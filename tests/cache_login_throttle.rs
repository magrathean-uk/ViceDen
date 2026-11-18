use chrono::{Duration, Utc};
use viceden::cache::login_throttle::LoginThrottle;

#[tokio::test]
async fn allows_until_threshold_then_blocks_in_window() {
    let throttle = LoginThrottle::new(5, Duration::minutes(15));
    let key = "ip|root";

    for _ in 0..5 {
        assert!(throttle.allowed(key).await);
        throttle.record_failure(key).await;
    }
    assert!(!throttle.allowed(key).await);
}

#[tokio::test]
async fn clears_on_success() {
    let throttle = LoginThrottle::new(3, Duration::minutes(15));
    let key = "ip|alice";

    for _ in 0..3 {
        throttle.record_failure(key).await;
    }
    assert!(!throttle.allowed(key).await);

    throttle.clear(key).await;
    assert!(throttle.allowed(key).await);
}

#[tokio::test]
async fn forgets_outside_window() {
    let throttle = LoginThrottle::new(3, Duration::milliseconds(20));
    let key = "ip|bob";
    for _ in 0..3 {
        throttle.record_failure(key).await;
    }
    assert!(!throttle.allowed(key).await);

    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    assert!(throttle.allowed(key).await);
}

#[tokio::test]
async fn prune_drops_old_keys() {
    let throttle = LoginThrottle::new(3, Duration::milliseconds(20));
    throttle.record_failure("ip|gone").await;
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    throttle.prune(Utc::now() - Duration::milliseconds(20)).await;
    assert!(throttle.allowed("ip|gone").await);
}
