use std::time::Duration as StdDuration;
use uuid::Uuid;
use viceden::cache::mention_suggest::MentionSuggest;

#[tokio::test]
async fn returns_inserted_handles() {
    let cache = MentionSuggest::new(StdDuration::from_secs(60));
    let thread = Uuid::new_v4();
    cache.set(thread, vec!["alice".to_string(), "bob".to_string()]).await;

    let got = cache.get(thread).await;
    assert_eq!(got, Some(vec!["alice".to_string(), "bob".to_string()]));
}

#[tokio::test]
async fn expires_after_ttl() {
    let cache = MentionSuggest::new(StdDuration::from_millis(20));
    let thread = Uuid::new_v4();
    cache.set(thread, vec!["alice".to_string()]).await;

    tokio::time::sleep(StdDuration::from_millis(40)).await;
    assert_eq!(cache.get(thread).await, None);
}

#[tokio::test]
async fn invalidate_removes_entry() {
    let cache = MentionSuggest::new(StdDuration::from_secs(60));
    let thread = Uuid::new_v4();
    cache.set(thread, vec!["alice".to_string()]).await;
    cache.invalidate(thread).await;
    assert_eq!(cache.get(thread).await, None);
}
