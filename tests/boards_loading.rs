use std::path::PathBuf;

use viceden::models::Board;

#[test]
fn boards_file_loads_and_contains_expected_core_boards() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let boards_path = manifest_dir.join("config").join("boards.toml");

    let boards = Board::load_from_file(&boards_path).expect("boards config should load");

    assert!(!boards.is_empty(), "boards list should not be empty");
    assert_eq!(boards.len(), 12, "board list should stay compact");
    assert!(boards.iter().any(|board| board.slug == "general"));
    assert!(boards.iter().any(|board| board.slug == "hobbies"));
    assert!(boards.iter().any(|board| board.slug == "stories"));
    assert!(boards.iter().any(|board| board.slug == "wellbeing"));
    assert!(boards.iter().any(|board| board.slug == "meetups"));
    assert!(boards.iter().any(|board| board.slug == "setbacks"));
    assert!(boards.iter().any(|board| board.slug == "mindfulness"));
    assert!(boards.iter().any(|board| board.slug == "projects"));
    assert!(boards.iter().any(|board| board.slug == "debate"));
    assert!(boards.iter().any(|board| board.slug == "careers"));
    assert!(boards.iter().all(|board| board.slug != "darklife"));
    assert!(boards.iter().all(|board| board.slug != "privacy"));
    assert!(boards.iter().all(|board| board.slug != "recovery"));
    assert!(boards.iter().all(|board| board.slug != "culture"));
    assert!(boards.iter().all(|board| board.ttl_days == 180));
}

#[test]
fn board_description_helper_reports_empty_and_non_empty_values() {
    let board_without_description = Board {
        slug: "test".to_string(),
        name: "Test".to_string(),
        ttl_days: 180,
        description: String::new(),
    };
    let board_with_description =
        Board { description: "desc".to_string(), ..board_without_description.clone() };

    assert!(!board_without_description.has_description());
    assert!(board_with_description.has_description());
}
