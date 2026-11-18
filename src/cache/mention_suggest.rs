use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone)]
pub struct MentionSuggest {
    inner: Arc<RwLock<HashMap<Uuid, Entry>>>,
    ttl: Duration,
}

struct Entry {
    handles: Vec<String>,
    expires_at: Instant,
}

impl MentionSuggest {
    pub fn new(ttl: Duration) -> Self {
        Self { inner: Arc::new(RwLock::new(HashMap::new())), ttl }
    }

    pub async fn get(&self, thread_id: Uuid) -> Option<Vec<String>> {
        let now = Instant::now();
        let guard = self.inner.read().await;
        let entry = guard.get(&thread_id)?;
        if entry.expires_at < now {
            return None;
        }
        Some(entry.handles.clone())
    }

    pub async fn set(&self, thread_id: Uuid, handles: Vec<String>) {
        let entry = Entry { handles, expires_at: Instant::now() + self.ttl };
        let mut guard = self.inner.write().await;
        guard.insert(thread_id, entry);
    }

    pub async fn invalidate(&self, thread_id: Uuid) {
        let mut guard = self.inner.write().await;
        guard.remove(&thread_id);
    }
}
