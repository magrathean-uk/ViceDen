use uuid::Uuid;
use viceden::{
    forum::{ForumStore, SearchScope, ROOT_USER_ID},
    models::Board,
};

fn boards() -> Vec<Board> {
    let boards_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config").join("boards.toml");
    Board::load_from_file(&boards_path).expect("boards config should load")
}

fn store() -> ForumStore {
    let state_file = std::env::temp_dir().join(format!("viceden-next-{}.json", Uuid::new_v4()));
    ForumStore::with_state_path(&boards(), &state_file)
}

#[test]
fn trust_thread_states_spam_edit_mute_invites_metrics_and_search_filters_work() {
    let mut store = store();
    let member = store
        .create_user("newmember", "secret123", Some("127.0.0.10".to_string()))
        .expect("member should be created");

    assert_eq!(store.trust_level_for_user(member.id), 0);
    assert!(store.trust_level_for_user(ROOT_USER_ID) >= 3);

    let invite = store.issue_invite(ROOT_USER_ID, "friend").expect("invite should issue");
    let invited = store
        .redeem_invite(&invite.code, "invitee", "secret123", Some("127.0.0.11".to_string()))
        .expect("invite should create user");
    assert!(store.invite_summary().iter().any(|item| item.redeemed_by == Some(invited.id)));
    store.follow_tag(member.id, "safety").expect("tag follow should work");
    let tag_notifications_before = store.notifications_for_user(member.id).len();

    let spam_body =
        (0..9).map(|idx| format!("https://example.com/{idx}")).collect::<Vec<_>>().join(" ");
    assert!(
        store
            .create_thread_with_checks("general", "too many links".to_string(), spam_body, member.id)
            .is_err(),
        "spam guard should reject link floods"
    );

    let thread = store
        .create_thread_with_tags(
            "general",
            "stateful topic".to_string(),
            "first body".to_string(),
            ROOT_USER_ID,
            vec!["Safety".to_string()],
        )
        .expect("thread should be created");
    assert!(
        store.notifications_for_user(member.id).len() > tag_notifications_before,
        "followed tag should notify on new tagged thread"
    );
    assert!(
        store.notifications_for_user(member.id).iter().any(|item| item.message.contains("#safety")),
        "tag notification should name the followed tag"
    );
    store.set_thread_pinned(thread.id, ROOT_USER_ID, true).expect("pin should work");
    store.set_thread_answered(thread.id, ROOT_USER_ID, true).expect("answer should work");
    store.set_thread_locked(thread.id, ROOT_USER_ID, true).expect("lock should work");
    let state = store.thread_state(thread.id);
    assert!(state.pinned);
    assert!(state.answered);
    assert!(state.locked);

    assert!(
        store.create_post_with_checks(thread.id, "member reply".to_string(), member.id).is_err(),
        "locked thread should block member replies"
    );

    let (_, posts) = store.thread_with_posts(&thread.id).expect("posts should exist");
    let root_post = posts[0].id;
    store
        .edit_post(root_post, ROOT_USER_ID, "edited first body".to_string())
        .expect("author can edit own post");
    assert_eq!(store.post_edit_history(root_post).len(), 1);

    store.mute_user(ROOT_USER_ID, member.id).expect("mute should work");
    store.set_thread_locked(thread.id, ROOT_USER_ID, false).expect("unlock should work");
    let before = store.notifications_for_user(ROOT_USER_ID).len();
    store
        .create_post_with_checks(thread.id, "@root hello".to_string(), member.id)
        .expect("member can post after unlock");
    assert_eq!(
        store.notifications_for_user(ROOT_USER_ID).len(),
        before,
        "muted actor should not notify"
    );

    let hits = store.search(ROOT_USER_ID, "tag:safety author:root", SearchScope::All, true, 20);
    assert!(hits.iter().any(|hit| hit.thread_id == thread.id));

    let metrics = store.admin_metrics();
    assert!(metrics.total_users >= 3);
    assert!(metrics.total_threads >= 1);
    assert!(metrics.total_posts >= 1);
}
