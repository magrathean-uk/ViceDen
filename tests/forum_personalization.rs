use uuid::Uuid;
use viceden::{
    forum::{ForumStore, SearchScope, ROOT_USER_ID},
    models::Board,
};

fn boards() -> Vec<Board> {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let boards_path = manifest_dir.join("config").join("boards.toml");
    Board::load_from_file(&boards_path).expect("boards config should load")
}

#[test]
fn subscriptions_save_and_search_work_together() {
    let boards = boards();
    let state_file = std::env::temp_dir().join(format!("viceden-forum-{}.json", Uuid::new_v4()));
    let mut store = ForumStore::with_state_path(&boards, &state_file);

    let starting_subscriptions = store.subscribed_board_slugs_for_user(ROOT_USER_ID);
    assert!(!starting_subscriptions.is_empty(), "root should start with seeded subscriptions");

    let board_slug = boards.first().expect("at least one board").slug.clone();
    let was_subscribed = store.is_board_subscribed(ROOT_USER_ID, &board_slug);
    let now_subscribed =
        store.toggle_board_subscription(ROOT_USER_ID, &board_slug).expect("toggle should work");
    assert_ne!(was_subscribed, now_subscribed, "subscription state should flip");

    let unique = format!("rust-search-{}", Uuid::new_v4());
    let title = format!("{unique} title");
    let body = format!("body contains {unique} and enough text to search later");

    let thread = store
        .create_thread_with_checks(&board_slug, title.clone(), body.clone(), ROOT_USER_ID)
        .expect("thread should be created");

    let search_hits = store.search(ROOT_USER_ID, &unique, SearchScope::All, true, 20);
    assert!(
        search_hits.iter().any(|hit| hit.thread_id == thread.id),
        "search should find the created thread"
    );

    let scoped_hits = store.search(
        ROOT_USER_ID,
        &format!("{unique} : /{board_slug}"),
        SearchScope::All,
        true,
        20,
    );
    assert!(
        scoped_hits.iter().any(|hit| hit.thread_id == thread.id),
        "scoped search should find thread in matching board"
    );

    let other_board_slug = boards
        .iter()
        .find(|board| board.slug != board_slug)
        .expect("need another board")
        .slug
        .clone();
    let wrong_scope_hits = store.search(
        ROOT_USER_ID,
        &format!("{unique} : /{other_board_slug}"),
        SearchScope::All,
        true,
        20,
    );
    assert!(
        wrong_scope_hits.iter().all(|hit| hit.thread_id != thread.id),
        "scoped search should exclude hits from other boards"
    );

    let saved =
        store.toggle_thread_saved(ROOT_USER_ID, thread.id).expect("save toggle should work");
    assert!(saved, "thread should become saved");
    assert!(
        store.saved_threads_for_user(ROOT_USER_ID).iter().any(|item| item.id == thread.id),
        "saved thread list should contain the thread"
    );

    let duplicate = store.create_thread_with_checks(&board_slug, title, body, ROOT_USER_ID);
    assert!(duplicate.is_err(), "duplicate title in same board should be rejected");

    let _ = std::fs::remove_file(state_file);
}

#[test]
fn thread_tags_are_normalized_deduped_and_limited() {
    let boards = boards();
    let state_file = std::env::temp_dir().join(format!("viceden-tags-{}.json", Uuid::new_v4()));
    let mut store = ForumStore::with_state_path(&boards, &state_file);

    let thread = store
        .create_thread_with_tags(
            "general",
            "tagged thread".to_string(),
            "body".to_string(),
            ROOT_USER_ID,
            vec![
                "Safety".to_string(),
                "safety".to_string(),
                "Long Tag Name".to_string(),
                "third".to_string(),
                "fourth".to_string(),
            ],
        )
        .expect("thread should be created");

    assert_eq!(
        store.tags_for_thread(thread.id),
        vec!["safety".to_string(), "long-tag-name".to_string(), "third".to_string()]
    );

    let _ = std::fs::remove_file(state_file);
}
