use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct LoginThrottle {
    inner: Arc<RwLock<HashMap<String, Vec<DateTime<Utc>>>>>,
    max_failures: usize,
    window: Duration,
}

impl LoginThrottle {
    pub fn new(max_failures: usize, window: Duration) -> Self {
        Self { inner: Arc::new(RwLock::new(HashMap::new())), max_failures, window }
    }

    pub async fn allowed(&self, key: &str) -> bool {
        let cutoff = Utc::now() - self.window;
        let mut guard = self.inner.write().await;
        if let Some(timestamps) = guard.get_mut(key) {
            timestamps.retain(|ts| *ts > cutoff);
            if timestamps.is_empty() {
                guard.remove(key);
                return true;
            }
            return timestamps.len() < self.max_failures;
        }
        true
    }

    pub async fn record_failure(&self, key: &str) {
        let mut guard = self.inner.write().await;
        guard.entry(key.to_string()).or_default().push(Utc::now());
    }

    pub async fn clear(&self, key: &str) {
        let mut guard = self.inner.write().await;
        guard.remove(key);
    }

    pub async fn prune(&self, threshold: DateTime<Utc>) {
        let mut guard = self.inner.write().await;
        guard.retain(|_, timestamps| {
            timestamps.retain(|ts| *ts > threshold);
            !timestamps.is_empty()
        });
    }
}
