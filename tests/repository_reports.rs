//! Integration tests for the reports and moderation-events repositories.
//!
//! These tests require `VICEDEN_DATABASE__URL` to point at a Postgres instance
//! with the viceden migrations applied. They are marked `#[ignore]` so the
//! default `cargo test` run stays green when no local PG is available.

mod common;

use chrono::{Duration, Utc};
use std::path::PathBuf;
use uuid::Uuid;
use viceden::repository::moderation_events::{ModerationEventInsert, ModerationEventRepo};
use viceden::repository::posts::{ModState, NewPost, PostRepo};
use viceden::repository::reports::{NewReport, ReportQueueFilter, ReportRepo};
use viceden::repository::threads::{NewThread, ThreadRepo};
use viceden::repository::users::{NewUser, UserRepo};

async fn insert_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    prefix: &str,
    role: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    let prefix = prefix.chars().take(7).collect::<String>();
    UserRepo
        .insert(
            &mut **tx,
            NewUser {
                id,
                handle: format!("{prefix}-{}", &id.simple().to_string()[..8]),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: role.into(),
                last_known_ip: None,
            },
        )
        .await
        .expect("seed user");
    id
}

async fn insert_thread(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    title: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    ThreadRepo
        .insert(
            &mut **tx,
            NewThread {
                id,
                board_slug: "general".into(),
                user_id,
                title: title.into(),
                expires_at: Utc::now() + Duration::days(30),
            },
        )
        .await
        .expect("seed thread");
    id
}

async fn insert_post(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: Uuid,
    user_id: Uuid,
    body: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    PostRepo
        .insert(
            &mut **tx,
            NewPost { id, thread_id, user_id, body: body.into(), mod_state: ModState::Visible },
        )
        .await
        .expect("seed post");
    id
}

async fn seed_boards(pool: &sqlx::PgPool) {
    let boards_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config").join("boards.toml");
    let boards = viceden::models::Board::load_from_file(&boards_path).expect("boards load");
    viceden::db::sync_boards(pool, &boards).await.expect("seed boards");
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with viceden migrations applied"]
async fn list_queue_orders_open_reports_by_priority_then_age() {
    let pool = common::fresh_pool().await;
    seed_boards(&pool).await;
    let mut tx = pool.begin().await.unwrap();

    let founder = insert_user(&mut tx, "founder", "founder").await;
    let author = insert_user(&mut tx, "author", "member").await;
    let reporter = insert_user(&mut tx, "reporter", "member").await;
    let thread = insert_thread(&mut tx, author, "priority thread").await;
    let post = insert_post(&mut tx, thread, author, "body").await;

    let urgent_id = Uuid::new_v4();
    ReportRepo
        .insert(
            &mut *tx,
            NewReport {
                id: urgent_id,
                actor_id: reporter,
                target_post_id: Some(post),
                target_thread_id: Some(thread),
                reason: "minor risk".into(),
                details: Some("urgent".into()),
                created_at: Utc::now() - Duration::minutes(5),
            },
        )
        .await
        .unwrap();
    let normal_id = Uuid::new_v4();
    ReportRepo
        .insert(
            &mut *tx,
            NewReport {
                id: normal_id,
                actor_id: reporter,
                target_post_id: Some(post),
                target_thread_id: Some(thread),
                reason: "unclear".into(),
                details: None,
                created_at: Utc::now() - Duration::minutes(1),
            },
        )
        .await
        .unwrap();
    let spam_id = Uuid::new_v4();
    ReportRepo
        .insert(
            &mut *tx,
            NewReport {
                id: spam_id,
                actor_id: reporter,
                target_post_id: Some(post),
                target_thread_id: Some(thread),
                reason: "spam links".into(),
                details: None,
                created_at: Utc::now() - Duration::minutes(2),
            },
        )
        .await
        .unwrap();

    let queue = ReportRepo.list_queue(&mut *tx).await.unwrap();
    assert_eq!(queue.len(), 3);
    assert_eq!(queue[0].id, urgent_id);
    assert_eq!(queue[0].priority_label, "urgent");
    assert_eq!(queue[0].target_user_id, Some(author));
    assert_eq!(queue[0].reviewer_user_id, None);
    assert_eq!(queue[1].id, spam_id);
    assert_eq!(queue[2].id, normal_id);

    ReportRepo
        .resolve_with_note(&mut *tx, urgent_id, founder, true, Some("handled"))
        .await
        .unwrap();
    let queue_after = ReportRepo.list_queue(&mut *tx).await.unwrap();
    assert_eq!(queue_after.len(), 2);

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with viceden migrations applied"]
async fn report_resolution_and_audit_rows_round_trip() {
    let pool = common::fresh_pool().await;
    seed_boards(&pool).await;
    let mut tx = pool.begin().await.unwrap();

    let founder = insert_user(&mut tx, "founder", "founder").await;
    let author = insert_user(&mut tx, "author", "member").await;
    let reporter = insert_user(&mut tx, "reporter", "member").await;
    let thread = insert_thread(&mut tx, author, "audit thread").await;
    let post = insert_post(&mut tx, thread, author, "body").await;
    let report_id = Uuid::new_v4();

    ReportRepo
        .insert(
            &mut *tx,
            NewReport {
                id: report_id,
                actor_id: reporter,
                target_post_id: Some(post),
                target_thread_id: Some(thread),
                reason: "doxxing".into(),
                details: Some("contains personal detail".into()),
                created_at: Utc::now(),
            },
        )
        .await
        .unwrap();

    ReportRepo
        .resolve_with_note(&mut *tx, report_id, founder, true, Some("removed and warned"))
        .await
        .unwrap();
    ModerationEventRepo
        .insert(
            &mut *tx,
            ModerationEventInsert {
                id: Uuid::new_v4(),
                actor_id: founder,
                target_post_id: Some(post),
                target_thread_id: Some(thread),
                target_user_id: Some(author),
                report_id: Some(report_id),
                action: "remove_post".into(),
                reason: "doxxing".into(),
                note: Some("removed and warned".into()),
                created_at: Utc::now(),
            },
        )
        .await
        .unwrap();

    let report = ReportRepo.find_by_id(&mut *tx, report_id).await.unwrap().unwrap();
    assert_eq!(report.reviewed_by, Some(founder));
    assert!(report.reviewed_at.is_some());
    assert_eq!(report.resolution_note.as_deref(), Some("removed and warned"));
    assert!(report.priority >= 1);

    let audit = ModerationEventRepo.list_for_report(&mut *tx, report_id).await.unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].actor_id, founder);
    assert_eq!(audit[0].target_user_id, Some(author));
    assert_eq!(audit[0].action, "remove_post");
    assert_eq!(audit[0].note.as_deref(), Some("removed and warned"));

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with viceden migrations applied"]
async fn list_queue_filtered_supports_owner_and_history_views() {
    let pool = common::fresh_pool().await;
    seed_boards(&pool).await;
    let mut tx = pool.begin().await.unwrap();

    let founder = insert_user(&mut tx, "founder", "founder").await;
    let author = insert_user(&mut tx, "author", "member").await;
    let reporter = insert_user(&mut tx, "reporter", "member").await;
    let open_thread = insert_thread(&mut tx, author, "open thread").await;
    let resolved_thread = insert_thread(&mut tx, author, "resolved thread").await;

    let open_id = Uuid::new_v4();
    ReportRepo
        .insert(
            &mut *tx,
            NewReport {
                id: open_id,
                actor_id: reporter,
                target_post_id: None,
                target_thread_id: Some(open_thread),
                reason: "open review".into(),
                details: None,
                created_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    ReportRepo.triage(&mut *tx, open_id, Some("reviewing"), Some(Some(founder))).await.unwrap();

    let resolved_id = Uuid::new_v4();
    ReportRepo
        .insert(
            &mut *tx,
            NewReport {
                id: resolved_id,
                actor_id: reporter,
                target_post_id: None,
                target_thread_id: Some(resolved_thread),
                reason: "resolved review".into(),
                details: None,
                created_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    ReportRepo.resolve_with_note(&mut *tx, resolved_id, founder, true, Some("done")).await.unwrap();

    let mine = ReportRepo
        .list_queue_filtered(
            &mut *tx,
            ReportQueueFilter {
                assigned: Some("me"),
                queue_state: Some("reviewing"),
                viewer_id: Some(founder),
                ..ReportQueueFilter::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].id, open_id);

    let history = ReportRepo
        .list_queue_filtered(
            &mut *tx,
            ReportQueueFilter { status: Some("resolved"), ..ReportQueueFilter::default() },
        )
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, resolved_id);
    assert_eq!(history[0].resolution_note.as_deref(), Some("done"));

    tx.rollback().await.unwrap();
}
