//! Database utilities and initialisation.
//!
//! Keep this module small.  Its job is to initialise the database and run
//! migrations, not to become an ORM god object.

use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tracing::warn;
use uuid::Uuid;

const FORUM_STATE_KEY: &str = "default";
const DB_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(3);
const DB_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const HOT_QUERY_WARN_AFTER: Duration = Duration::from_millis(5);

fn smallint_for_db(value: i8) -> i16 {
    i16::from(value)
}

fn user_settings_compactness_for_db(value: i8) -> i16 {
    smallint_for_db(value)
}

fn redact_database_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return "<redacted>".to_string();
    };
    let credentials_start = scheme_end + 3;
    let Some(at_offset) = url[credentials_start..].find('@') else {
        return url.to_string();
    };
    let credentials_end = credentials_start + at_offset;
    format!("{}<credentials>{}", &url[..credentials_start], &url[credentials_end..])
}

fn token_storage_digest(token: &str, pepper: Option<&str>) -> (String, &'static str) {
    let digest = crate::auth::tokens::digest_token(token, pepper);
    (digest.for_storage(), digest.version_label())
}

fn token_lookup_digests(token: &str, pepper: Option<&str>) -> Vec<String> {
    crate::auth::tokens::lookup_digests(token, pepper)
}

fn notification_kind_label(kind: &crate::forum::NotificationKind) -> &'static str {
    match kind {
        crate::forum::NotificationKind::Mention => "mention",
        crate::forum::NotificationKind::Reply => "reply",
        crate::forum::NotificationKind::PostRemoved => "post_removed",
        crate::forum::NotificationKind::Report => "report",
        crate::forum::NotificationKind::ModAction => "mod_action",
        crate::forum::NotificationKind::Invite => "invite",
    }
}

fn notification_kind_from_label(label: &str) -> crate::forum::NotificationKind {
    match label {
        "reply" => crate::forum::NotificationKind::Reply,
        "post_removed" => crate::forum::NotificationKind::PostRemoved,
        "report" => crate::forum::NotificationKind::Report,
        "mod_action" => crate::forum::NotificationKind::ModAction,
        "invite" => crate::forum::NotificationKind::Invite,
        _ => crate::forum::NotificationKind::Mention,
    }
}

/// Apply embedded SQLx migrations.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::migrate!("./migrations").run(pool).await.map_err(Into::into)
}

pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}

/// Connect to PostgreSQL and apply migrations.
///
/// Postgres is mandatory. Connection and migration failures bubble up and
/// startup must stop instead of falling back to local JSON state.
pub async fn connect_pool(
    settings: &crate::config::DatabaseSettings,
) -> Result<PgPool, anyhow::Error> {
    if matches!(settings.url.trim(), "" | "disabled" | "local" | "off") {
        anyhow::bail!("database connection failed: VICEDEN_DATABASE__URL must point at Postgres");
    }

    let pool = PgPoolOptions::new()
        .max_connections(settings.max_connections)
        .acquire_timeout(DB_ACQUIRE_TIMEOUT)
        .idle_timeout(DB_IDLE_TIMEOUT)
        .connect(&settings.url)
        .await
        .with_context(|| {
            format!("database connection failed: {}", redact_database_url(&settings.url))
        })?;

    run_migrations(&pool).await.context("database migrations failed")?;

    Ok(pool)
}

pub async fn load_forum_runtime_state(pool: &PgPool) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>(
        "SELECT payload::text FROM forum_runtime_state WHERE state_key = $1",
    )
    .bind(FORUM_STATE_KEY)
    .fetch_optional(pool)
    .await
}

pub async fn save_forum_runtime_state(pool: &PgPool, payload: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO forum_runtime_state (state_key, payload, updated_at)
        VALUES ($1, $2::jsonb, now())
        ON CONFLICT (state_key)
        DO UPDATE SET payload = EXCLUDED.payload, updated_at = now()
        "#,
    )
    .bind(FORUM_STATE_KEY)
    .bind(payload)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn sync_boards(
    pool: &PgPool,
    boards: &[crate::models::Board],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for board in boards {
        sqlx::query(
            r#"
            INSERT INTO boards (slug, name, ttl_days, description, downvotes_enabled)
            VALUES ($1, $2, $3, $4, TRUE)
            ON CONFLICT (slug) DO UPDATE
            SET name = EXCLUDED.name,
                ttl_days = EXCLUDED.ttl_days,
                description = EXCLUDED.description
            "#,
        )
        .bind(&board.slug)
        .bind(&board.name)
        .bind(board.ttl_days as i32)
        .bind(&board.description)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn ensure_bootstrap_users(pool: &PgPool) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now();
    let users = [
        (
            crate::forum::ROOT_USER_ID,
            crate::forum::ROOT_HANDLE,
            crate::auth::hash_password(crate::forum::ROOT_HANDLE),
            "founder",
            now,
        ),
        (
            Uuid::from_u128(1001),
            "atlas",
            crate::auth::hash_password("atlas"),
            "member",
            now - chrono::Duration::hours(8),
        ),
        (
            Uuid::from_u128(1002),
            "luna",
            crate::auth::hash_password("luna"),
            "member",
            now - chrono::Duration::hours(8),
        ),
        (
            Uuid::from_u128(1003),
            "jade",
            crate::auth::hash_password("jade"),
            "member",
            now - chrono::Duration::hours(8),
        ),
        (
            Uuid::from_u128(1004),
            "viper",
            crate::auth::hash_password("viper"),
            "member",
            now - chrono::Duration::hours(8),
        ),
        (
            Uuid::from_u128(1005),
            "modbot",
            crate::auth::hash_password("modbot"),
            "mod",
            now - chrono::Duration::hours(8),
        ),
    ];

    for (id, handle, password_hash, role, created_at) in users {
        sqlx::query(
            r#"
            INSERT INTO users (id, handle, password_hash, role, created_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(id)
        .bind(handle)
        .bind(password_hash)
        .bind(role)
        .bind(created_at)
        .execute(pool)
        .await?;
    }

    Ok(())
}

pub async fn founder_uses_default_password(pool: &PgPool) -> Result<bool, sqlx::Error> {
    let Some(row) =
        crate::repository::users::UserRepo.find_by_id(pool, crate::forum::ROOT_USER_ID).await?
    else {
        return Ok(false);
    };
    Ok(crate::auth::verify_password(&row.handle, crate::forum::ROOT_HANDLE, &row.password_hash))
}

pub async fn set_founder_password(pool: &PgPool, password: &str) -> Result<(), anyhow::Error> {
    let password = password.trim();
    if password.len() < 8 {
        anyhow::bail!("founder password must be at least 8 characters");
    }

    let result = sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(crate::auth::hash_password(password))
        .bind(crate::forum::ROOT_USER_ID)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        anyhow::bail!("founder account not found");
    }

    Ok(())
}

pub async fn sync_users(
    pool: &PgPool,
    users: &HashMap<Uuid, crate::forum::ForumUserRecord>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    let user_ids = users.keys().copied().collect::<Vec<_>>();
    sqlx::query("DELETE FROM users WHERE NOT (id = ANY($1))")
        .bind(&user_ids)
        .execute(&mut *tx)
        .await?;

    for user in users.values() {
        sqlx::query(
            r#"
            INSERT INTO users (
                id,
                handle,
                password_hash,
                created_at,
                banned,
                role,
                probation_until,
                last_known_ip
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (id) DO UPDATE
            SET handle = EXCLUDED.handle,
                password_hash = EXCLUDED.password_hash,
                created_at = EXCLUDED.created_at,
                banned = EXCLUDED.banned,
                role = EXCLUDED.role,
                probation_until = EXCLUDED.probation_until,
                last_known_ip = EXCLUDED.last_known_ip
            "#,
        )
        .bind(user.id)
        .bind(&user.handle)
        .bind(&user.password_hash)
        .bind(user.created_at)
        .bind(user.is_banned)
        .bind(match user.role {
            crate::models::UserRole::Founder => "founder",
            crate::models::UserRole::Mod => "mod",
            crate::models::UserRole::Member => "member",
        })
        .bind(user.probation_until)
        .bind(&user.last_known_ip)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn upsert_user(
    pool: &PgPool,
    user: &crate::forum::ForumUserRecord,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO users (
            id,
            handle,
            password_hash,
            created_at,
            banned,
            role,
            probation_until,
            last_known_ip
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (id) DO UPDATE
        SET handle = EXCLUDED.handle,
            password_hash = EXCLUDED.password_hash,
            created_at = EXCLUDED.created_at,
            banned = EXCLUDED.banned,
            role = EXCLUDED.role,
            probation_until = EXCLUDED.probation_until,
            last_known_ip = EXCLUDED.last_known_ip
        "#,
    )
    .bind(user.id)
    .bind(&user.handle)
    .bind(&user.password_hash)
    .bind(user.created_at)
    .bind(user.is_banned)
    .bind(match user.role {
        crate::models::UserRole::Founder => "founder",
        crate::models::UserRole::Mod => "mod",
        crate::models::UserRole::Member => "member",
    })
    .bind(user.probation_until)
    .bind(&user.last_known_ip)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn load_session(
    pool: &PgPool,
    token: &str,
    token_pepper: Option<&str>,
) -> Result<Option<crate::forum::SessionRecord>, sqlx::Error> {
    let digests = token_lookup_digests(token, token_pepper);
    let row = sqlx::query(
        r#"
        SELECT user_id, created_at, expires_at, csrf_token
        FROM sessions
        WHERE token_hash = ANY($1)
        ORDER BY CASE token_digest_version
            WHEN 'hmac-sha256' THEN 0
            WHEN 'sha256' THEN 1
            ELSE 2
        END
        LIMIT 1
        "#,
    )
    .bind(&digests)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| crate::forum::SessionRecord {
        user_id: row.get("user_id"),
        created_at: row.get("created_at"),
        expires_at: row.get("expires_at"),
        csrf_token: row.get("csrf_token"),
    }))
}

pub async fn insert_session(
    pool: &PgPool,
    token: &str,
    token_pepper: Option<&str>,
    session: &crate::forum::SessionRecord,
) -> Result<(), sqlx::Error> {
    let (digest, version_label) = token_storage_digest(token, token_pepper);
    sqlx::query(
        r#"
        INSERT INTO sessions (token_hash, token_digest_version, user_id, created_at, expires_at, csrf_token)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(digest)
    .bind(version_label)
    .bind(session.user_id)
    .bind(session.created_at)
    .bind(session.expires_at)
    .bind(&session.csrf_token)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_session(
    pool: &PgPool,
    token: &str,
    token_pepper: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let digests = token_lookup_digests(token, token_pepper);
    let result = sqlx::query("DELETE FROM sessions WHERE token_hash = ANY($1)")
        .bind(&digests)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn resolve_api_token_user(
    pool: &PgPool,
    token: &str,
    token_pepper: Option<&str>,
) -> Result<Option<Uuid>, sqlx::Error> {
    let digests = token_lookup_digests(token, token_pepper);
    let row = sqlx::query(
        r#"
        UPDATE api_tokens
        SET last_used_at = now()
        WHERE token_hash = ANY($1)
        RETURNING user_id
        "#,
    )
    .bind(&digests)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| row.get("user_id")))
}

pub async fn list_api_tokens_for_user(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<crate::forum::ApiTokenSummary>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT id, name, preview, created_at, last_used_at
        FROM api_tokens
        WHERE user_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| crate::forum::ApiTokenSummary {
            id: row.get("id"),
            name: row.get("name"),
            preview: row.get("preview"),
            created_at: row.get::<DateTime<Utc>, _>("created_at"),
            last_used_at: row.get("last_used_at"),
        })
        .collect())
}

pub async fn insert_api_token(
    pool: &PgPool,
    token: &str,
    token_pepper: Option<&str>,
    record: &crate::forum::ApiTokenRecord,
) -> Result<(), sqlx::Error> {
    let (digest, version_label) = token_storage_digest(token, token_pepper);
    sqlx::query(
        r#"
        INSERT INTO api_tokens (token_hash, token_digest_version, id, user_id, name, preview, created_at, last_used_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(digest)
    .bind(version_label)
    .bind(record.id)
    .bind(record.user_id)
    .bind(&record.name)
    .bind(&record.preview)
    .bind(record.created_at)
    .bind(record.last_used_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_api_token(
    pool: &PgPool,
    user_id: Uuid,
    token: &str,
    token_pepper: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let digests = token_lookup_digests(token, token_pepper);
    let result = sqlx::query("DELETE FROM api_tokens WHERE user_id = $1 AND token_hash = ANY($2)")
        .bind(user_id)
        .bind(&digests)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_api_token_by_id(
    pool: &PgPool,
    user_id: Uuid,
    token_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM api_tokens WHERE user_id = $1 AND id = $2")
        .bind(user_id)
        .bind(token_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub struct NormalizedForumData {
    pub users: HashMap<Uuid, crate::forum::ForumUserRecord>,
    pub threads: HashMap<Uuid, crate::models::Thread>,
    pub posts: HashMap<Uuid, Vec<crate::models::Post>>,
    pub notifications: HashMap<Uuid, Vec<crate::forum::ForumNotification>>,
    pub user_settings: HashMap<Uuid, crate::forum::UserProfileSettings>,
    pub board_subscriptions: HashMap<Uuid, HashSet<String>>,
    pub hidden_threads: HashMap<Uuid, HashSet<Uuid>>,
    pub saved_threads: HashMap<Uuid, HashSet<Uuid>>,
    pub votes: HashMap<Uuid, HashMap<Uuid, i8>>,
    pub reports: Vec<crate::forum::ReportRecord>,
    pub moderation_events: Vec<crate::forum::ModerationEvent>,
    pub thread_tags: HashMap<Uuid, Vec<String>>,
    pub thread_states: HashMap<Uuid, crate::forum::ThreadState>,
    pub post_edits: HashMap<Uuid, Vec<crate::forum::PostEditRecord>>,
    pub muted_users: HashMap<Uuid, HashSet<Uuid>>,
    pub tag_subscriptions: HashMap<Uuid, HashSet<String>>,
    pub invites: HashMap<String, crate::forum::InviteRecord>,
    pub banned_ips: HashSet<String>,
}

pub async fn load_normalized_forum_data(pool: &PgPool) -> Result<NormalizedForumData, sqlx::Error> {
    let user_rows = sqlx::query(
        r#"
        SELECT id, handle, password_hash, role, banned, created_at, probation_until, last_known_ip
        FROM users
        "#,
    )
    .fetch_all(pool)
    .await?;
    let users = user_rows
        .into_iter()
        .map(|row| {
            let role: String = row.get("role");
            let role = match role.as_str() {
                "founder" => crate::models::UserRole::Founder,
                "mod" => crate::models::UserRole::Mod,
                _ => crate::models::UserRole::Member,
            };
            let user = crate::forum::ForumUserRecord {
                id: row.get("id"),
                handle: row.get("handle"),
                password_hash: row.get("password_hash"),
                role,
                is_banned: row.get("banned"),
                probation_until: row.get("probation_until"),
                created_at: row.get("created_at"),
                last_known_ip: row.get("last_known_ip"),
            };
            (user.id, user)
        })
        .collect();

    let threads = load_all_threads(pool)
        .await?
        .into_iter()
        .map(|thread| (thread.id, thread))
        .collect::<HashMap<_, _>>();

    let post_rows = sqlx::query(
        r#"
        SELECT id, thread_id, user_id, body, created_at, state
        FROM posts
        ORDER BY created_at ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    let mut posts: HashMap<Uuid, Vec<crate::models::Post>> = HashMap::new();
    for row in post_rows {
        let thread_id: Uuid = row.get("thread_id");
        posts.entry(thread_id).or_default().push(crate::models::Post {
            id: row.get("id"),
            thread_id,
            user_id: row.get("user_id"),
            body: row.get("body"),
            created_at: row.get("created_at"),
            state: match row.get::<i32, _>("state") {
                1 => crate::models::PostState::Quarantine,
                2 => crate::models::PostState::Reject,
                _ => crate::models::PostState::Allow,
            },
        });
    }

    let setting_rows = sqlx::query(
        "SELECT user_id, compactness, theme, notify_mentions, notify_replies, notify_moderation FROM user_settings",
    )
        .fetch_all(pool)
        .await?;
    let user_settings = setting_rows
        .into_iter()
        .map(|row| {
            (
                row.get("user_id"),
                crate::forum::UserProfileSettings {
                    compactness: row.get::<i16, _>("compactness") as i8,
                    theme: row.get("theme"),
                    notify_mentions: row.get("notify_mentions"),
                    notify_replies: row.get("notify_replies"),
                    notify_moderation: row.get("notify_moderation"),
                },
            )
        })
        .collect::<HashMap<_, _>>();

    let subscription_rows =
        sqlx::query("SELECT user_id, board_slug FROM board_subscriptions").fetch_all(pool).await?;
    let mut board_subscriptions: HashMap<Uuid, HashSet<String>> = HashMap::new();
    for row in subscription_rows {
        board_subscriptions.entry(row.get("user_id")).or_default().insert(row.get("board_slug"));
    }

    let hidden_rows =
        sqlx::query("SELECT user_id, thread_id FROM hidden_threads").fetch_all(pool).await?;
    let mut hidden_threads: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
    for row in hidden_rows {
        hidden_threads.entry(row.get("user_id")).or_default().insert(row.get("thread_id"));
    }

    let saved_rows =
        sqlx::query("SELECT user_id, thread_id FROM saved_threads").fetch_all(pool).await?;
    let mut saved_threads: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
    for row in saved_rows {
        saved_threads.entry(row.get("user_id")).or_default().insert(row.get("thread_id"));
    }

    let vote_rows =
        sqlx::query("SELECT post_id, user_id, value FROM votes").fetch_all(pool).await?;
    let mut votes: HashMap<Uuid, HashMap<Uuid, i8>> = HashMap::new();
    for row in vote_rows {
        votes
            .entry(row.get("post_id"))
            .or_default()
            .insert(row.get("user_id"), row.get::<i16, _>("value") as i8);
    }

    let notification_rows = sqlx::query(
        r#"
        SELECT recipient_user_id, id, actor_id, kind, thread_id, thread_title, post_id, message, created_at, is_read
        FROM notifications
        ORDER BY created_at ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    let mut notifications: HashMap<Uuid, Vec<crate::forum::ForumNotification>> = HashMap::new();
    for row in notification_rows {
        let recipient_user_id: Uuid = row.get("recipient_user_id");
        let kind: String = row.get("kind");
        notifications.entry(recipient_user_id).or_default().push(crate::forum::ForumNotification {
            id: row.get("id"),
            actor_id: row.get("actor_id"),
            kind: notification_kind_from_label(&kind),
            thread_id: row.get("thread_id"),
            thread_title: row.get("thread_title"),
            post_id: row.get("post_id"),
            message: row.get("message"),
            created_at: row.get("created_at"),
            is_read: row.get("is_read"),
        });
    }

    let reports = list_reports(pool).await?;
    let thread_tags = load_thread_tags(pool).await?;
    let thread_states = load_thread_states(pool).await?;
    let post_edits = load_post_edits(pool).await?;
    let muted_users = load_muted_users(pool).await?;
    let tag_subscriptions = load_tag_subscriptions(pool).await?;
    let invites = load_invites(pool).await?;

    let moderation_rows = sqlx::query(
        r#"
        SELECT id, actor_id, target_post_id, target_thread_id, reason, created_at
        FROM moderation_events
        ORDER BY created_at ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    let moderation_events = moderation_rows
        .into_iter()
        .map(|row| crate::forum::ModerationEvent {
            id: row.get("id"),
            actor_id: row.get("actor_id"),
            target_post_id: row.get("target_post_id"),
            target_thread_id: row.get("target_thread_id"),
            reason: row.get("reason"),
            created_at: row.get("created_at"),
        })
        .collect::<Vec<_>>();

    let banned_ip_rows = sqlx::query("SELECT ip FROM banned_ips").fetch_all(pool).await?;
    let banned_ips = banned_ip_rows.into_iter().map(|row| row.get("ip")).collect();

    Ok(NormalizedForumData {
        users,
        threads,
        posts,
        notifications,
        user_settings,
        board_subscriptions,
        hidden_threads,
        saved_threads,
        votes,
        reports,
        moderation_events,
        thread_tags,
        thread_states,
        post_edits,
        muted_users,
        tag_subscriptions,
        invites,
        banned_ips,
    })
}

pub async fn load_thread_tags(pool: &PgPool) -> Result<HashMap<Uuid, Vec<String>>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT thread_id, tag
        FROM thread_tags
        ORDER BY thread_id, position ASC, tag ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut tags = HashMap::new();
    for row in rows {
        tags.entry(row.get("thread_id")).or_insert_with(Vec::new).push(row.get("tag"));
    }
    Ok(tags)
}

pub async fn load_thread_states(
    pool: &PgPool,
) -> Result<HashMap<Uuid, crate::forum::ThreadState>, sqlx::Error> {
    let rows = sqlx::query("SELECT thread_id, pinned, locked, answered, closed FROM thread_states")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.get("thread_id"),
                crate::forum::ThreadState {
                    pinned: row.get("pinned"),
                    locked: row.get("locked"),
                    answered: row.get("answered"),
                    closed: row.get("closed"),
                },
            )
        })
        .collect())
}

pub async fn load_post_edits(
    pool: &PgPool,
) -> Result<HashMap<Uuid, Vec<crate::forum::PostEditRecord>>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT post_id, editor_id, previous_body, new_body, edited_at
        FROM post_edits
        ORDER BY edited_at ASC
        "#,
    )
    .fetch_all(pool)
    .await?;
    let mut edits = HashMap::new();
    for row in rows {
        let post_id = row.get("post_id");
        edits.entry(post_id).or_insert_with(Vec::new).push(crate::forum::PostEditRecord {
            post_id,
            editor_id: row.get("editor_id"),
            previous_body: row.get("previous_body"),
            new_body: row.get("new_body"),
            edited_at: row.get("edited_at"),
        });
    }
    Ok(edits)
}

pub async fn load_muted_users(pool: &PgPool) -> Result<HashMap<Uuid, HashSet<Uuid>>, sqlx::Error> {
    let rows =
        sqlx::query("SELECT user_id, muted_user_id FROM muted_users").fetch_all(pool).await?;
    let mut muted = HashMap::new();
    for row in rows {
        muted
            .entry(row.get("user_id"))
            .or_insert_with(HashSet::new)
            .insert(row.get("muted_user_id"));
    }
    Ok(muted)
}

pub async fn load_tag_subscriptions(
    pool: &PgPool,
) -> Result<HashMap<Uuid, HashSet<String>>, sqlx::Error> {
    let rows = sqlx::query("SELECT user_id, tag FROM tag_subscriptions").fetch_all(pool).await?;
    let mut subscriptions = HashMap::new();
    for row in rows {
        subscriptions.entry(row.get("user_id")).or_insert_with(HashSet::new).insert(row.get("tag"));
    }
    Ok(subscriptions)
}

pub async fn load_invites(
    pool: &PgPool,
) -> Result<HashMap<String, crate::forum::InviteRecord>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT code, label, inviter_id, created_at, redeemed_by, redeemed_at
        FROM invites
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let invite = crate::forum::InviteRecord {
                code: row.get("code"),
                label: row.get("label"),
                inviter_id: row.get("inviter_id"),
                created_at: row.get("created_at"),
                redeemed_by: row.get("redeemed_by"),
                redeemed_at: row.get("redeemed_at"),
            };
            (invite.code.clone(), invite)
        })
        .collect())
}

pub async fn sync_preference_tables(
    pool: &PgPool,
    user_settings: &std::collections::HashMap<Uuid, crate::forum::UserProfileSettings>,
    board_subscriptions: &std::collections::HashMap<Uuid, std::collections::HashSet<String>>,
    hidden_threads: &std::collections::HashMap<Uuid, std::collections::HashSet<Uuid>>,
    saved_threads: &std::collections::HashMap<Uuid, std::collections::HashSet<Uuid>>,
    votes: &std::collections::HashMap<Uuid, std::collections::HashMap<Uuid, i8>>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    for (user_id, settings) in user_settings {
        sqlx::query(
            r#"
            INSERT INTO user_settings (
                user_id, compactness, theme, notify_mentions, notify_replies, notify_moderation
            )
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (user_id) DO UPDATE
            SET compactness = EXCLUDED.compactness,
                theme = EXCLUDED.theme,
                notify_mentions = EXCLUDED.notify_mentions,
                notify_replies = EXCLUDED.notify_replies,
                notify_moderation = EXCLUDED.notify_moderation
            "#,
        )
        .bind(*user_id)
        .bind(user_settings_compactness_for_db(settings.compactness))
        .bind(&settings.theme)
        .bind(settings.notify_mentions)
        .bind(settings.notify_replies)
        .bind(settings.notify_moderation)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query("DELETE FROM board_subscriptions").execute(&mut *tx).await?;
    for (user_id, slugs) in board_subscriptions {
        for slug in slugs {
            sqlx::query("INSERT INTO board_subscriptions (user_id, board_slug) VALUES ($1, $2)")
                .bind(*user_id)
                .bind(slug)
                .execute(&mut *tx)
                .await?;
        }
    }

    sqlx::query("DELETE FROM hidden_threads").execute(&mut *tx).await?;
    for (user_id, thread_ids) in hidden_threads {
        for thread_id in thread_ids {
            sqlx::query("INSERT INTO hidden_threads (user_id, thread_id) VALUES ($1, $2)")
                .bind(*user_id)
                .bind(*thread_id)
                .execute(&mut *tx)
                .await?;
        }
    }

    sqlx::query("DELETE FROM saved_threads").execute(&mut *tx).await?;
    for (user_id, thread_ids) in saved_threads {
        for thread_id in thread_ids {
            sqlx::query("INSERT INTO saved_threads (user_id, thread_id) VALUES ($1, $2)")
                .bind(*user_id)
                .bind(*thread_id)
                .execute(&mut *tx)
                .await?;
        }
    }

    sqlx::query("DELETE FROM votes").execute(&mut *tx).await?;
    for (post_id, post_votes) in votes {
        for (user_id, value) in post_votes {
            sqlx::query("INSERT INTO votes (post_id, user_id, value) VALUES ($1, $2, $3)")
                .bind(*post_id)
                .bind(*user_id)
                .bind(smallint_for_db(*value))
                .execute(&mut *tx)
                .await?;
        }
    }

    tx.commit().await?;
    Ok(())
}

pub async fn upsert_user_settings(
    pool: &PgPool,
    user_id: Uuid,
    settings: &crate::forum::UserProfileSettings,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO user_settings (
            user_id, compactness, theme, notify_mentions, notify_replies, notify_moderation
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (user_id) DO UPDATE
        SET compactness = EXCLUDED.compactness,
            theme = EXCLUDED.theme,
            notify_mentions = EXCLUDED.notify_mentions,
            notify_replies = EXCLUDED.notify_replies,
            notify_moderation = EXCLUDED.notify_moderation
        "#,
    )
    .bind(user_id)
    .bind(user_settings_compactness_for_db(settings.compactness))
    .bind(&settings.theme)
    .bind(settings.notify_mentions)
    .bind(settings.notify_replies)
    .bind(settings.notify_moderation)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_thread_tags(
    pool: &PgPool,
    thread_id: Uuid,
    tags: &[String],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM thread_tags WHERE thread_id = $1")
        .bind(thread_id)
        .execute(&mut *tx)
        .await?;
    for (position, tag) in tags.iter().enumerate() {
        sqlx::query(
            r#"
            INSERT INTO thread_tags (thread_id, tag, position)
            VALUES ($1, $2, $3)
            ON CONFLICT (thread_id, tag) DO UPDATE
            SET position = EXCLUDED.position
            "#,
        )
        .bind(thread_id)
        .bind(tag)
        .bind(position as i32)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn sync_thread_tags(
    pool: &PgPool,
    thread_tags: &std::collections::HashMap<Uuid, Vec<String>>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM thread_tags").execute(&mut *tx).await?;
    for (thread_id, tags) in thread_tags {
        for (position, tag) in tags.iter().enumerate() {
            sqlx::query(
                r#"
                INSERT INTO thread_tags (thread_id, tag, position)
                VALUES ($1, $2, $3)
                "#,
            )
            .bind(*thread_id)
            .bind(tag)
            .bind(position as i32)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

pub async fn sync_next_feature_tables(
    pool: &PgPool,
    thread_states: &std::collections::HashMap<Uuid, crate::forum::ThreadState>,
    post_edits: &std::collections::HashMap<Uuid, Vec<crate::forum::PostEditRecord>>,
    muted_users: &std::collections::HashMap<Uuid, std::collections::HashSet<Uuid>>,
    tag_subscriptions: &std::collections::HashMap<Uuid, std::collections::HashSet<String>>,
    invites: &std::collections::HashMap<String, crate::forum::InviteRecord>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM thread_states").execute(&mut *tx).await?;
    for (thread_id, state) in thread_states {
        sqlx::query(
            r#"
            INSERT INTO thread_states (thread_id, pinned, locked, answered, closed)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(*thread_id)
        .bind(state.pinned)
        .bind(state.locked)
        .bind(state.answered)
        .bind(state.closed)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query("DELETE FROM post_edits").execute(&mut *tx).await?;
    for edits in post_edits.values() {
        for edit in edits {
            sqlx::query(
                r#"
                INSERT INTO post_edits (post_id, editor_id, previous_body, new_body, edited_at)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (post_id, edited_at) DO NOTHING
                "#,
            )
            .bind(edit.post_id)
            .bind(edit.editor_id)
            .bind(&edit.previous_body)
            .bind(&edit.new_body)
            .bind(edit.edited_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    sqlx::query("DELETE FROM muted_users").execute(&mut *tx).await?;
    for (user_id, muted) in muted_users {
        for muted_user_id in muted {
            sqlx::query("INSERT INTO muted_users (user_id, muted_user_id) VALUES ($1, $2)")
                .bind(*user_id)
                .bind(*muted_user_id)
                .execute(&mut *tx)
                .await?;
        }
    }

    sqlx::query("DELETE FROM tag_subscriptions").execute(&mut *tx).await?;
    for (user_id, tags) in tag_subscriptions {
        for tag in tags {
            sqlx::query("INSERT INTO tag_subscriptions (user_id, tag) VALUES ($1, $2)")
                .bind(*user_id)
                .bind(tag)
                .execute(&mut *tx)
                .await?;
        }
    }

    sqlx::query("DELETE FROM invites").execute(&mut *tx).await?;
    for invite in invites.values() {
        sqlx::query(
            r#"
            INSERT INTO invites (code, label, inviter_id, created_at, redeemed_by, redeemed_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(&invite.code)
        .bind(&invite.label)
        .bind(invite.inviter_id)
        .bind(invite.created_at)
        .bind(invite.redeemed_by)
        .bind(invite.redeemed_at)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn set_board_subscription(
    pool: &PgPool,
    user_id: Uuid,
    board_slug: &str,
    subscribed: bool,
) -> Result<(), sqlx::Error> {
    if subscribed {
        sqlx::query(
            r#"
            INSERT INTO board_subscriptions (user_id, board_slug)
            VALUES ($1, $2)
            ON CONFLICT (user_id, board_slug) DO NOTHING
            "#,
        )
        .bind(user_id)
        .bind(board_slug)
        .execute(pool)
        .await?;
    } else {
        sqlx::query("DELETE FROM board_subscriptions WHERE user_id = $1 AND board_slug = $2")
            .bind(user_id)
            .bind(board_slug)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn set_hidden_thread(
    pool: &PgPool,
    user_id: Uuid,
    thread_id: Uuid,
    hidden: bool,
) -> Result<(), sqlx::Error> {
    if hidden {
        sqlx::query(
            r#"
            INSERT INTO hidden_threads (user_id, thread_id)
            VALUES ($1, $2)
            ON CONFLICT (user_id, thread_id) DO NOTHING
            "#,
        )
        .bind(user_id)
        .bind(thread_id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query("DELETE FROM hidden_threads WHERE user_id = $1 AND thread_id = $2")
            .bind(user_id)
            .bind(thread_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn set_saved_thread(
    pool: &PgPool,
    user_id: Uuid,
    thread_id: Uuid,
    saved: bool,
) -> Result<(), sqlx::Error> {
    if saved {
        sqlx::query(
            r#"
            INSERT INTO saved_threads (user_id, thread_id)
            VALUES ($1, $2)
            ON CONFLICT (user_id, thread_id) DO NOTHING
            "#,
        )
        .bind(user_id)
        .bind(thread_id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query("DELETE FROM saved_threads WHERE user_id = $1 AND thread_id = $2")
            .bind(user_id)
            .bind(thread_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn set_vote(
    pool: &PgPool,
    post_id: Uuid,
    user_id: Uuid,
    value: Option<i8>,
) -> Result<(), sqlx::Error> {
    if let Some(value) = value {
        sqlx::query(
            r#"
            INSERT INTO votes (post_id, user_id, value)
            VALUES ($1, $2, $3)
            ON CONFLICT (post_id, user_id) DO UPDATE
            SET value = EXCLUDED.value
            "#,
        )
        .bind(post_id)
        .bind(user_id)
        .bind(smallint_for_db(value))
        .execute(pool)
        .await?;
    } else {
        sqlx::query("DELETE FROM votes WHERE post_id = $1 AND user_id = $2")
            .bind(post_id)
            .bind(user_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn sync_moderation_tables(
    pool: &PgPool,
    notifications: &std::collections::HashMap<Uuid, Vec<crate::forum::ForumNotification>>,
    reports: &[crate::forum::ReportRecord],
    moderation_events: &[crate::forum::ModerationEvent],
    banned_ips: &std::collections::HashSet<String>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM notifications").execute(&mut *tx).await?;
    for (recipient_user_id, items) in notifications {
        for item in items {
            sqlx::query(
                r#"
                INSERT INTO notifications (
                    id,
                    recipient_user_id,
                    actor_id,
                    kind,
                    thread_id,
                    thread_title,
                    post_id,
                    message,
                    created_at,
                    is_read
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                "#,
            )
            .bind(item.id)
            .bind(*recipient_user_id)
            .bind(item.actor_id)
            .bind(notification_kind_label(&item.kind))
            .bind(item.thread_id)
            .bind(&item.thread_title)
            .bind(item.post_id)
            .bind(&item.message)
            .bind(item.created_at)
            .bind(item.is_read)
            .execute(&mut *tx)
            .await?;
        }
    }

    sqlx::query("DELETE FROM reports").execute(&mut *tx).await?;
    for item in reports {
        sqlx::query(
            r#"
            INSERT INTO reports (
                id,
                actor_id,
                target_post_id,
                target_thread_id,
                reason,
                details,
                created_at,
                resolved
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(item.id)
        .bind(item.actor_id)
        .bind(item.target_post_id)
        .bind(item.target_thread_id)
        .bind(&item.reason)
        .bind(&item.details)
        .bind(item.created_at)
        .bind(item.resolved)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query("DELETE FROM moderation_events").execute(&mut *tx).await?;
    for item in moderation_events {
        sqlx::query(
            r#"
            INSERT INTO moderation_events (
                id,
                actor_id,
                target_post_id,
                target_thread_id,
                reason,
                created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(item.id)
        .bind(item.actor_id)
        .bind(item.target_post_id)
        .bind(item.target_thread_id)
        .bind(&item.reason)
        .bind(item.created_at)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query("DELETE FROM banned_ips").execute(&mut *tx).await?;
    for ip in banned_ips {
        sqlx::query("INSERT INTO banned_ips (ip) VALUES ($1)").bind(ip).execute(&mut *tx).await?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn list_notifications_for_user(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<crate::forum::ForumNotification>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT id, actor_id, kind, thread_id, thread_title, post_id, message, created_at, is_read
        FROM notifications
        WHERE recipient_user_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let kind: String = row.get("kind");
            crate::forum::ForumNotification {
                id: row.get("id"),
                actor_id: row.get("actor_id"),
                kind: notification_kind_from_label(&kind),
                thread_id: row.get("thread_id"),
                thread_title: row.get("thread_title"),
                post_id: row.get("post_id"),
                message: row.get("message"),
                created_at: row.get("created_at"),
                is_read: row.get("is_read"),
            }
        })
        .collect())
}

pub async fn insert_notification(
    pool: &PgPool,
    recipient_user_id: Uuid,
    item: &crate::forum::ForumNotification,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO notifications (
            id,
            recipient_user_id,
            actor_id,
            kind,
            thread_id,
            thread_title,
            post_id,
            message,
            created_at,
            is_read
        )
        SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10
        WHERE CASE
            WHEN $4 = 'mention' THEN COALESCE(
                (SELECT notify_mentions FROM user_settings WHERE user_id = $2),
                TRUE
            )
            WHEN $4 = 'reply' THEN COALESCE(
                (SELECT notify_replies FROM user_settings WHERE user_id = $2),
                TRUE
            )
            WHEN $4 IN ('post_removed', 'report', 'mod_action') THEN COALESCE(
                (SELECT notify_moderation FROM user_settings WHERE user_id = $2),
                TRUE
            )
            ELSE TRUE
        END
        "#,
    )
    .bind(item.id)
    .bind(recipient_user_id)
    .bind(item.actor_id)
    .bind(notification_kind_label(&item.kind))
    .bind(item.thread_id)
    .bind(&item.thread_title)
    .bind(item.post_id)
    .bind(&item.message)
    .bind(item.created_at)
    .bind(item.is_read)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn count_unread_notifications(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<usize, sqlx::Error> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM notifications WHERE recipient_user_id = $1 AND is_read = FALSE",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(count as usize)
}

pub async fn update_notification_read(
    pool: &PgPool,
    user_id: Uuid,
    notification_id: Uuid,
    read: bool,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE notifications
        SET is_read = $3
        WHERE recipient_user_id = $1 AND id = $2
        "#,
    )
    .bind(user_id)
    .bind(notification_id)
    .bind(read)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_notification(
    pool: &PgPool,
    user_id: Uuid,
    notification_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM notifications WHERE recipient_user_id = $1 AND id = $2")
        .bind(user_id)
        .bind(notification_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn mark_all_notifications_read(pool: &PgPool, user_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE notifications SET is_read = TRUE WHERE recipient_user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_reports(pool: &PgPool) -> Result<Vec<crate::forum::ReportRecord>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT id, actor_id, target_post_id, target_thread_id, reason, details, created_at, resolved
        FROM reports
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| crate::forum::ReportRecord {
            id: row.get("id"),
            actor_id: row.get("actor_id"),
            target_post_id: row.get("target_post_id"),
            target_thread_id: row.get("target_thread_id"),
            reason: row.get("reason"),
            details: row.get("details"),
            created_at: row.get("created_at"),
            resolved: row.get("resolved"),
        })
        .collect())
}

pub async fn insert_report(
    pool: &PgPool,
    item: &crate::forum::ReportRecord,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO reports (
            id,
            actor_id,
            target_post_id,
            target_thread_id,
            reason,
            details,
            created_at,
            resolved
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(item.id)
    .bind(item.actor_id)
    .bind(item.target_post_id)
    .bind(item.target_thread_id)
    .bind(&item.reason)
    .bind(&item.details)
    .bind(item.created_at)
    .bind(item.resolved)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_report_resolved(
    pool: &PgPool,
    report_id: Uuid,
    resolved: bool,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE reports SET resolved = $2 WHERE id = $1")
        .bind(report_id)
        .bind(resolved)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_admin_user_summaries(
    pool: &PgPool,
) -> Result<Vec<crate::forum::AdminUserSummary>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            users.id,
            users.handle,
            users.role,
            users.banned,
            COALESCE(thread_counts.thread_count, 0) AS thread_count,
            COALESCE(post_counts.post_count, 0) AS post_count
        FROM users
        LEFT JOIN (
            SELECT user_id, COUNT(*) AS thread_count
            FROM threads
            GROUP BY user_id
        ) AS thread_counts
            ON thread_counts.user_id = users.id
        LEFT JOIN (
            SELECT user_id, COUNT(*) AS post_count
            FROM posts
            GROUP BY user_id
        ) AS post_counts
            ON post_counts.user_id = users.id
        ORDER BY CASE WHEN users.role = 'founder' THEN 0 ELSE 1 END, users.handle ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let role: String = row.get("role");
            crate::forum::AdminUserSummary {
                id: row.get("id"),
                handle: row.get("handle"),
                role: role.clone(),
                is_banned: row.get("banned"),
                thread_count: row.get::<i64, _>("thread_count") as usize,
                post_count: row.get::<i64, _>("post_count") as usize,
                can_manage: role != "founder",
            }
        })
        .collect())
}

pub async fn sync_content_tables(
    pool: &PgPool,
    threads: &std::collections::HashMap<Uuid, crate::models::Thread>,
    posts: &std::collections::HashMap<Uuid, Vec<crate::models::Post>>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM posts").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM threads").execute(&mut *tx).await?;

    let mut ordered_threads = threads.values().cloned().collect::<Vec<_>>();
    ordered_threads.sort_by_key(|thread| thread.created_at);
    for thread in ordered_threads {
        sqlx::query(
            r#"
            INSERT INTO threads (id, board_slug, user_id, title, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(thread.id)
        .bind(&thread.board_slug)
        .bind(thread.user_id)
        .bind(&thread.title)
        .bind(thread.created_at)
        .bind(thread.expires_at)
        .execute(&mut *tx)
        .await?;
    }

    let mut ordered_posts =
        posts.values().flat_map(|items| items.iter().cloned()).collect::<Vec<_>>();
    ordered_posts.sort_by_key(|post| post.created_at);
    for post in ordered_posts {
        sqlx::query(
            r#"
            INSERT INTO posts (id, thread_id, user_id, body, created_at, state)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(post.id)
        .bind(post.thread_id)
        .bind(post.user_id)
        .bind(&post.body)
        .bind(post.created_at)
        .bind(post.state as i32)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Insert a thread + its root post in a single transaction.
///
/// `mod_pending=true` lands both rows with `mod_state='pending'`, which
/// is the fail-closed default once both Postgres and the moderation
/// pipeline are wired. The pipeline will flip them to `'visible'` (or to
/// `'quarantined'` / `'removed'`) once the classifier returns. Pass
/// `false` only for tests or maintenance paths that intentionally bypass the
/// classifier and insert already-visible rows.
pub async fn insert_thread_with_post(
    pool: &PgPool,
    thread: &crate::models::Thread,
    post: &crate::models::Post,
    mod_pending: bool,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let mod_state = if mod_pending { "pending" } else { "visible" };
    sqlx::query(
        r#"
        INSERT INTO threads (id, board_slug, user_id, title, created_at, expires_at,
                             mod_state, mod_state_changed_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, now())
        "#,
    )
    .bind(thread.id)
    .bind(&thread.board_slug)
    .bind(thread.user_id)
    .bind(&thread.title)
    .bind(thread.created_at)
    .bind(thread.expires_at)
    .bind(mod_state)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO posts (id, thread_id, user_id, body, created_at, state,
                           mod_state, mod_state_changed_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, now())
        "#,
    )
    .bind(post.id)
    .bind(post.thread_id)
    .bind(post.user_id)
    .bind(&post.body)
    .bind(post.created_at)
    .bind(post.state as i32)
    .bind(mod_state)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// See `insert_thread_with_post` for the `mod_pending` semantics.
pub async fn insert_post(
    pool: &PgPool,
    post: &crate::models::Post,
    mod_pending: bool,
) -> Result<(), sqlx::Error> {
    let mod_state = if mod_pending { "pending" } else { "visible" };
    sqlx::query(
        r#"
        INSERT INTO posts (id, thread_id, user_id, body, created_at, state,
                           mod_state, mod_state_changed_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, now())
        "#,
    )
    .bind(post.id)
    .bind(post.thread_id)
    .bind(post.user_id)
    .bind(&post.body)
    .bind(post.created_at)
    .bind(post.state as i32)
    .bind(mod_state)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_post(pool: &PgPool, post_id: Uuid) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM votes WHERE post_id = $1").bind(post_id).execute(&mut *tx).await?;
    let result =
        sqlx::query("DELETE FROM posts WHERE id = $1").bind(post_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_thread(pool: &PgPool, thread_id: Uuid) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"
        DELETE FROM votes
        WHERE post_id IN (SELECT id FROM posts WHERE thread_id = $1)
        "#,
    )
    .bind(thread_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM hidden_threads WHERE thread_id = $1")
        .bind(thread_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM saved_threads WHERE thread_id = $1")
        .bind(thread_id)
        .execute(&mut *tx)
        .await?;
    let result =
        sqlx::query("DELETE FROM threads WHERE id = $1").bind(thread_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected() > 0)
}

pub async fn insert_moderation_event(
    pool: &PgPool,
    item: &crate::forum::ModerationEvent,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO moderation_events (
            id,
            actor_id,
            target_post_id,
            target_thread_id,
            reason,
            created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(item.id)
    .bind(item.actor_id)
    .bind(item.target_post_id)
    .bind(item.target_thread_id)
    .bind(&item.reason)
    .bind(item.created_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn load_all_threads(pool: &PgPool) -> Result<Vec<crate::models::Thread>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT id, board_slug, title, user_id, created_at, expires_at
        FROM threads
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| crate::models::Thread {
            id: row.get("id"),
            board_slug: row.get("board_slug"),
            title: row.get("title"),
            user_id: row.get("user_id"),
            created_at: row.get("created_at"),
            expires_at: row.get("expires_at"),
        })
        .collect())
}

pub async fn count_threads_for_board(
    pool: &PgPool,
    board_slug: &str,
) -> Result<usize, sqlx::Error> {
    let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM threads WHERE board_slug = $1")
        .bind(board_slug)
        .fetch_one(pool)
        .await?;
    Ok(count as usize)
}

pub async fn load_thread_with_posts(
    pool: &PgPool,
    thread_id: Uuid,
) -> Result<Option<(crate::models::Thread, Vec<crate::models::Post>)>, sqlx::Error> {
    let thread_row = sqlx::query(
        r#"
        SELECT id, board_slug, title, user_id, created_at, expires_at
        FROM threads
        WHERE id = $1
        "#,
    )
    .bind(thread_id)
    .fetch_optional(pool)
    .await?;

    let Some(thread_row) = thread_row else {
        return Ok(None);
    };

    let thread = crate::models::Thread {
        id: thread_row.get("id"),
        board_slug: thread_row.get("board_slug"),
        title: thread_row.get("title"),
        user_id: thread_row.get("user_id"),
        created_at: thread_row.get("created_at"),
        expires_at: thread_row.get("expires_at"),
    };

    let post_rows = sqlx::query(
        r#"
        SELECT id, thread_id, user_id, body, created_at, state
        FROM posts
        WHERE thread_id = $1
        ORDER BY created_at ASC
        "#,
    )
    .bind(thread_id)
    .fetch_all(pool)
    .await?;

    let posts = post_rows
        .into_iter()
        .map(|row| crate::models::Post {
            id: row.get("id"),
            thread_id: row.get("thread_id"),
            user_id: row.get("user_id"),
            body: row.get("body"),
            created_at: row.get("created_at"),
            state: match row.get::<i32, _>("state") {
                1 => crate::models::PostState::Quarantine,
                2 => crate::models::PostState::Reject,
                _ => crate::models::PostState::Allow,
            },
        })
        .collect();

    Ok(Some((thread, posts)))
}

pub async fn load_thread_title(
    pool: &PgPool,
    thread_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>("SELECT title FROM threads WHERE id = $1")
        .bind(thread_id)
        .fetch_optional(pool)
        .await
}

pub async fn count_visible_board_threads(
    pool: &PgPool,
    user_id: Uuid,
    board_slug: &str,
) -> Result<usize, sqlx::Error> {
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM threads t
        WHERE t.board_slug = $1
          AND NOT EXISTS (
            SELECT 1 FROM hidden_threads h WHERE h.user_id = $2 AND h.thread_id = t.id
          )
        "#,
    )
    .bind(board_slug)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(count as usize)
}

pub async fn list_board_thread_previews(
    pool: &PgPool,
    user_id: Uuid,
    board_slug: &str,
    sort: &str,
    limit: usize,
    offset: usize,
) -> Result<Vec<crate::forum::ThreadPreview>, sqlx::Error> {
    let started = std::time::Instant::now();
    let order = match sort {
        "top" => "score DESC, t.created_at DESC, t.id DESC",
        "hot" => {
            "(score + 1.0) / sqrt(greatest(extract(epoch from (now() - t.created_at)) / 3600.0, 0) + 1.0) DESC, t.created_at DESC, t.id DESC"
        }
        "active" => "last_activity DESC, t.created_at DESC, t.id DESC",
        _ => "t.created_at DESC, t.id DESC",
    };
    let unanswered = if sort == "unanswered" { "AND stats.reply_count = 0" } else { "" };
    let sql = format!(
        r#"
        SELECT t.id, t.board_slug, t.title, t.user_id, t.created_at, t.expires_at,
               stats.reply_count, stats.last_activity, stats.score
        FROM threads t
        JOIN LATERAL (
            SELECT GREATEST(COUNT(p.id) - 1, 0)::BIGINT AS reply_count,
                   COALESCE(MAX(p.created_at), t.created_at) AS last_activity,
                   COALESCE(SUM(v.value), 0)::BIGINT AS score
            FROM posts p
            LEFT JOIN votes v ON v.post_id = p.id
            WHERE p.thread_id = t.id
        ) stats ON true
        WHERE t.board_slug = $1
          AND NOT EXISTS (
            SELECT 1 FROM hidden_threads h WHERE h.user_id = $2 AND h.thread_id = t.id
          )
          {unanswered}
        ORDER BY {order}
        LIMIT $3 OFFSET $4
        "#
    );

    // `order` and `unanswered` are selected from fixed literals; request values remain bound.
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(board_slug)
        .bind(user_id)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(pool)
        .await?;
    warn_hot_query(
        "list_board_thread_previews",
        started.elapsed(),
        &[("board_slug", board_slug), ("sort", sort)],
    );

    Ok(rows
        .into_iter()
        .map(|row| {
            let thread = crate::models::Thread {
                id: row.get("id"),
                board_slug: row.get("board_slug"),
                title: row.get("title"),
                user_id: row.get("user_id"),
                created_at: row.get("created_at"),
                expires_at: row.get("expires_at"),
            };
            let created_at = thread.created_at;
            crate::forum::ThreadPreview {
                thread,
                score: row.get::<i64, _>("score") as i32,
                reply_count: row.get::<i64, _>("reply_count") as usize,
                score_hidden: crate::forum::is_score_hidden(created_at),
                last_activity: row.get("last_activity"),
            }
        })
        .collect())
}

pub async fn search_forum(
    pool: &PgPool,
    user_id: Uuid,
    query: &str,
    board_slug: Option<&str>,
    scope: crate::forum::SearchScope,
    include_hidden: bool,
    limit: usize,
) -> Result<Vec<crate::forum::SearchHit>, sqlx::Error> {
    let started = std::time::Instant::now();
    let include_threads =
        matches!(scope, crate::forum::SearchScope::All | crate::forum::SearchScope::Threads);
    let include_posts =
        matches!(scope, crate::forum::SearchScope::All | crate::forum::SearchScope::Posts);
    let mut selects = Vec::new();
    if include_threads {
        selects.push(
            r#"
            SELECT t.id AS thread_id,
                   NULL::uuid AS post_id,
                   t.board_slug,
                   t.title AS thread_title,
                   t.user_id AS author_id,
                   COALESCE(root.body, '') AS body_snippet,
                   t.created_at,
                   COALESCE(score.score, 0)::BIGINT AS score,
                   'thread' AS kind,
                   EXISTS (SELECT 1 FROM saved_threads s WHERE s.user_id = $2 AND s.thread_id = t.id) AS is_saved,
                   EXISTS (SELECT 1 FROM hidden_threads h WHERE h.user_id = $2 AND h.thread_id = t.id) AS is_hidden
            FROM threads t
            JOIN q ON true
            LEFT JOIN LATERAL (
                SELECT body FROM posts WHERE thread_id = t.id ORDER BY created_at ASC LIMIT 1
            ) root ON true
            LEFT JOIN LATERAL (
                SELECT COALESCE(SUM(v.value), 0)::BIGINT AS score
                FROM posts p
                LEFT JOIN votes v ON v.post_id = p.id
                WHERE p.thread_id = t.id
            ) score ON true
            WHERE ($3::text IS NULL OR t.board_slug = $3)
              AND ($4 OR NOT EXISTS (
                SELECT 1 FROM hidden_threads h WHERE h.user_id = $2 AND h.thread_id = t.id
              ))
              AND to_tsvector('english', t.title) @@ q.query
            "#,
        );
        selects.push(
            r#"
            SELECT t.id AS thread_id,
                   NULL::uuid AS post_id,
                   t.board_slug,
                   t.title AS thread_title,
                   t.user_id AS author_id,
                   COALESCE(root.body, '') AS body_snippet,
                   t.created_at,
                   COALESCE(score.score, 0)::BIGINT AS score,
                   'thread' AS kind,
                   EXISTS (SELECT 1 FROM saved_threads s WHERE s.user_id = $2 AND s.thread_id = t.id) AS is_saved,
                   EXISTS (SELECT 1 FROM hidden_threads h WHERE h.user_id = $2 AND h.thread_id = t.id) AS is_hidden
            FROM threads t
            JOIN (
                SELECT DISTINCT thread_id
                FROM thread_tags
                WHERE tag ILIKE '%' || $1 || '%'
            ) tag_match ON tag_match.thread_id = t.id
            LEFT JOIN LATERAL (
                SELECT body FROM posts WHERE thread_id = t.id ORDER BY created_at ASC LIMIT 1
            ) root ON true
            LEFT JOIN LATERAL (
                SELECT COALESCE(SUM(v.value), 0)::BIGINT AS score
                FROM posts p
                LEFT JOIN votes v ON v.post_id = p.id
                WHERE p.thread_id = t.id
            ) score ON true
            WHERE ($3::text IS NULL OR t.board_slug = $3)
              AND ($4 OR NOT EXISTS (
                SELECT 1 FROM hidden_threads h WHERE h.user_id = $2 AND h.thread_id = t.id
              ))
            "#,
        );
    }
    if include_posts {
        selects.push(
            r#"
            SELECT t.id AS thread_id,
                   p.id AS post_id,
                   t.board_slug,
                   t.title AS thread_title,
                   p.user_id AS author_id,
                   p.body AS body_snippet,
                   p.created_at,
                   COALESCE(SUM(v.value), 0)::BIGINT AS score,
                   'post' AS kind,
                   EXISTS (SELECT 1 FROM saved_threads s WHERE s.user_id = $2 AND s.thread_id = t.id) AS is_saved,
                   EXISTS (SELECT 1 FROM hidden_threads h WHERE h.user_id = $2 AND h.thread_id = t.id) AS is_hidden
            FROM posts p
            JOIN threads t ON t.id = p.thread_id
            JOIN q ON true
            LEFT JOIN votes v ON v.post_id = p.id
            WHERE ($3::text IS NULL OR t.board_slug = $3)
              AND ($4 OR NOT EXISTS (
                SELECT 1 FROM hidden_threads h WHERE h.user_id = $2 AND h.thread_id = t.id
              ))
              AND to_tsvector('english', p.body) @@ q.query
            GROUP BY t.id, p.id
            "#,
        );
    }

    if selects.is_empty() {
        return Ok(Vec::new());
    }

    let sql = format!(
        r#"
        WITH q AS (SELECT websearch_to_tsquery('english', $1) AS query)
        SELECT *
        FROM ({}) hits
        ORDER BY created_at DESC, score DESC, thread_title ASC
        LIMIT $5
        "#,
        selects.join(" UNION ALL ")
    );

    // `selects` contains fixed query templates; all client-supplied values remain bound.
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(query)
        .bind(user_id)
        .bind(board_slug)
        .bind(include_hidden)
        .bind(limit as i64)
        .fetch_all(pool)
        .await?;
    warn_hot_query(
        "search_forum",
        started.elapsed(),
        &[("scope", scope.as_str()), ("board_slug", board_slug.unwrap_or("*"))],
    );

    Ok(rows
        .into_iter()
        .map(|row| crate::forum::SearchHit {
            thread_id: row.get("thread_id"),
            post_id: row.get("post_id"),
            board_slug: row.get("board_slug"),
            thread_title: row.get("thread_title"),
            author_id: row.get("author_id"),
            body_snippet: crate::forum::search_snippet_for_display(
                &row.get::<String, _>("body_snippet"),
                220,
            ),
            created_at: row.get("created_at"),
            score: row.get::<i64, _>("score") as i32,
            kind: if row.get::<String, _>("kind") == "thread" {
                crate::forum::SearchHitKind::Thread
            } else {
                crate::forum::SearchHitKind::Post
            },
            is_saved: row.get("is_saved"),
            is_hidden: row.get("is_hidden"),
        })
        .collect())
}

fn warn_hot_query(query_name: &'static str, elapsed: Duration, labels: &[(&str, &str)]) {
    if elapsed > HOT_QUERY_WARN_AFTER {
        let elapsed_ms = elapsed.as_millis() as u64;
        warn!(query_name, elapsed_ms, ?labels, "slow_hot_query");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn user_settings_compactness_uses_postgres_smallint_type() {
        let compactness: i16 = super::user_settings_compactness_for_db(1);

        assert_eq!(compactness, 1);
    }

    #[test]
    fn database_url_redaction_hides_credentials() {
        let redacted = super::redact_database_url(
            "postgres://viceden:super-secret@127.0.0.1:5432/viceden?connect_timeout=1",
        );

        assert_eq!(redacted, "postgres://<credentials>@127.0.0.1:5432/viceden?connect_timeout=1");
        assert!(!redacted.contains("super-secret"));
    }
}
