use uuid::Uuid;
use viceden::{
    forum::{ForumStore, NotificationKind, ROOT_USER_ID},
    models::Board,
};

fn boards() -> Vec<Board> {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let boards_path = manifest_dir.join("config").join("boards.toml");
    Board::load_from_file(&boards_path).expect("boards config should load")
}

#[test]
fn founder_can_wipe_delete_users_and_remove_notifications() {
    let boards = boards();
    let state_file = std::env::temp_dir().join(format!("viceden-admin-{}.json", Uuid::new_v4()));
    let mut store = ForumStore::with_state_path(&boards, &state_file);

    let target_user_id = Uuid::from_u128(1001);
    let thread = store
        .all_threads()
        .into_iter()
        .find(|thread| thread.user_id == target_user_id)
        .expect("seeded target thread should exist");

    store.add_notification(
        ROOT_USER_ID,
        Some(target_user_id),
        NotificationKind::Mention,
        Some(thread.id),
        None,
        Some("temporary notification".to_string()),
    );
    let notification = store
        .notifications_for_user(ROOT_USER_ID)
        .into_iter()
        .find(|item| item.message == "temporary notification")
        .expect("notification should exist");

    store.set_last_known_ip(target_user_id, "198.51.100.24".to_string()).expect("ip should store");
    let wiped =
        store.wipe_user_content(ROOT_USER_ID, target_user_id).expect("founder should wipe user");
    assert_eq!(wiped.handle, "atlas");
    assert!(
        store
            .admin_user_summaries()
            .iter()
            .find(|user| user.id == target_user_id)
            .is_some_and(|user| !user.is_banned),
        "wipe should keep account record"
    );
    assert!(
        store.all_threads().into_iter().all(|thread| thread.user_id != target_user_id),
        "wiped user threads should be gone"
    );
    assert!(
        !store.notifications_for_user(ROOT_USER_ID).iter().any(|item| item.id == notification.id),
        "wipe should also remove the user's related notifications"
    );

    let recreated = store
        .create_thread_with_checks(
            &boards.first().expect("board exists").slug,
            format!("atlas-return-{}", Uuid::new_v4()),
            "body".to_string(),
            target_user_id,
        )
        .expect("user should still exist after wipe");
    assert_eq!(recreated.user_id, target_user_id);

    let deleted =
        store.delete_user(ROOT_USER_ID, target_user_id).expect("founder should delete user");
    assert_eq!(deleted.handle, "atlas");
    assert!(store.user_id_for_handle("atlas").is_none(), "deleted user should be gone");
    assert!(
        store.all_threads().into_iter().all(|thread| thread.user_id != target_user_id),
        "deleted user threads should be gone"
    );
    assert!(store.banned_ips().contains("198.51.100.24"), "delete should ban known IP");

    let _ = std::fs::remove_file(state_file);
}

#[test]
fn repeating_same_vote_clears_it() {
    let boards = boards();
    let state_file =
        std::env::temp_dir().join(format!("viceden-vote-toggle-{}.json", Uuid::new_v4()));
    let mut store = ForumStore::with_state_path(&boards, &state_file);

    let thread = store.all_threads().into_iter().next().expect("seeded thread should exist");
    let (_, posts) = store.thread_with_posts(&thread.id).expect("thread posts should exist");
    let post_id = posts[0].id;

    store.vote_post(post_id, ROOT_USER_ID, 1).expect("first upvote should work");
    assert_eq!(store.user_vote_for_post(post_id, ROOT_USER_ID), 1);

    store.vote_post(post_id, ROOT_USER_ID, 1).expect("same upvote should untap");
    assert_eq!(store.user_vote_for_post(post_id, ROOT_USER_ID), 0);

    let _ = std::fs::remove_file(state_file);
}
