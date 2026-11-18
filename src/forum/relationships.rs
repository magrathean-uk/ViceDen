use super::*;

impl ForumStore {
    pub fn subscribed_board_slugs_for_user(&self, user_id: Uuid) -> Vec<String> {
        let mut slugs = self
            .board_subscriptions
            .get(&user_id)
            .map(|items| items.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        slugs.sort();
        slugs
    }

    pub fn is_board_subscribed(&self, user_id: Uuid, board_slug: &str) -> bool {
        self.board_subscriptions.get(&user_id).is_some_and(|items| items.contains(board_slug))
    }

    pub fn toggle_board_subscription(
        &mut self,
        user_id: Uuid,
        board_slug: &str,
    ) -> Result<bool, ForumActionError> {
        if !self.boards_exist(board_slug) {
            return Err(ForumActionError::NotFound("board not found".to_string()));
        }

        let subscriptions = self.board_subscriptions.entry(user_id).or_default();
        let is_now_subscribed = if subscriptions.contains(board_slug) {
            subscriptions.remove(board_slug);
            false
        } else {
            subscriptions.insert(board_slug.to_string());
            true
        };

        if let Some(pool) = self.db_pool.clone() {
            let board_slug = board_slug.to_string();
            let board_slug_for_db = board_slug.clone();
            let updated = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::set_board_subscription(
                        &pool,
                        user_id,
                        &board_slug_for_db,
                        is_now_subscribed,
                    )
                    .await
                })
            });
            match updated {
                Ok(()) => {
                    self.persist_runtime_state();
                    return Ok(is_now_subscribed);
                }
                Err(error) => {
                    if self.requires_postgres() {
                        let subscriptions = self.board_subscriptions.entry(user_id).or_default();
                        if is_now_subscribed {
                            subscriptions.remove(&board_slug);
                        } else {
                            subscriptions.insert(board_slug.clone());
                        }
                        return Err(self.storage_error(format!(
                            "board subscription write in postgres failed: {error}"
                        )));
                    }
                    warn!(
                        error = %error,
                        "board subscription write in postgres failed; falling back to full sync"
                    );
                }
            }
        }
        self.persist_state();
        Ok(is_now_subscribed)
    }

    pub fn is_thread_hidden(&self, user_id: Uuid, thread_id: Uuid) -> bool {
        self.hidden_threads.get(&user_id).is_some_and(|items| items.contains(&thread_id))
    }

    pub fn toggle_thread_hidden(
        &mut self,
        user_id: Uuid,
        thread_id: Uuid,
    ) -> Result<bool, ForumActionError> {
        if !self.has_thread(&thread_id) {
            return Err(ForumActionError::NotFound("thread not found".to_string()));
        }

        let hidden = self.hidden_threads.entry(user_id).or_default();
        let is_now_hidden = if hidden.contains(&thread_id) {
            hidden.remove(&thread_id);
            false
        } else {
            hidden.insert(thread_id);
            true
        };

        if let Some(pool) = self.db_pool.clone() {
            let updated = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::set_hidden_thread(&pool, user_id, thread_id, is_now_hidden).await
                })
            });
            match updated {
                Ok(()) => {
                    self.persist_runtime_state();
                    return Ok(is_now_hidden);
                }
                Err(error) => {
                    if self.requires_postgres() {
                        let hidden = self.hidden_threads.entry(user_id).or_default();
                        if is_now_hidden {
                            hidden.remove(&thread_id);
                        } else {
                            hidden.insert(thread_id);
                        }
                        return Err(self.storage_error(format!(
                            "hidden thread write in postgres failed: {error}"
                        )));
                    }
                    warn!(
                        error = %error,
                        "hidden thread write in postgres failed; falling back to full sync"
                    );
                }
            }
        }
        self.persist_state();
        Ok(is_now_hidden)
    }

    pub fn is_thread_saved(&self, user_id: Uuid, thread_id: Uuid) -> bool {
        self.saved_threads.get(&user_id).is_some_and(|items| items.contains(&thread_id))
    }

    pub fn toggle_thread_saved(
        &mut self,
        user_id: Uuid,
        thread_id: Uuid,
    ) -> Result<bool, ForumActionError> {
        if !self.has_thread(&thread_id) {
            return Err(ForumActionError::NotFound("thread not found".to_string()));
        }

        let saved = self.saved_threads.entry(user_id).or_default();
        let is_now_saved = if saved.contains(&thread_id) {
            saved.remove(&thread_id);
            false
        } else {
            saved.insert(thread_id);
            true
        };

        if let Some(pool) = self.db_pool.clone() {
            let updated = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::set_saved_thread(&pool, user_id, thread_id, is_now_saved).await
                })
            });
            match updated {
                Ok(()) => {
                    self.persist_runtime_state();
                    return Ok(is_now_saved);
                }
                Err(error) => {
                    if self.requires_postgres() {
                        let saved = self.saved_threads.entry(user_id).or_default();
                        if is_now_saved {
                            saved.remove(&thread_id);
                        } else {
                            saved.insert(thread_id);
                        }
                        return Err(self.storage_error(format!(
                            "saved thread write in postgres failed: {error}"
                        )));
                    }
                    warn!(
                        error = %error,
                        "saved thread write in postgres failed; falling back to full sync"
                    );
                }
            }
        }
        self.persist_state();
        Ok(is_now_saved)
    }

    pub fn saved_threads_for_user(&self, user_id: Uuid) -> Vec<Thread> {
        let saved_ids = self.saved_threads.get(&user_id).cloned().unwrap_or_default();
        let mut threads: Vec<Thread> = self
            .all_threads()
            .into_iter()
            .filter(|thread| saved_ids.contains(&thread.id))
            .collect();
        threads.sort_by_key(|thread| thread.created_at);
        threads.reverse();
        threads
    }

    pub fn hidden_threads_for_user(&self, user_id: Uuid) -> Vec<Thread> {
        let hidden_ids = self.hidden_threads.get(&user_id).cloned().unwrap_or_default();
        let mut threads: Vec<Thread> = self
            .all_threads()
            .into_iter()
            .filter(|thread| hidden_ids.contains(&thread.id))
            .collect();
        threads.sort_by_key(|thread| thread.created_at);
        threads.reverse();
        threads
    }
}
