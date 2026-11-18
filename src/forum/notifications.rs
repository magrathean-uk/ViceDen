use super::*;

impl ForumStore {
    fn reply_notifications_enabled_for(&self, recipient_user_id: Uuid) -> bool {
        if let Some(pool) = self.db_pool.clone() {
            return block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::repository::preferences::PreferencesRepo
                        .settings_for(&pool, recipient_user_id)
                        .await
                        .map(|settings| settings.notify_replies)
                        .unwrap_or(false)
                })
            });
        }
        self.user_settings
            .get(&recipient_user_id)
            .map(|settings| settings.notify_replies)
            .unwrap_or(true)
    }

    fn notifications_enabled_for(&self, recipient_user_id: Uuid, kind: NotificationKind) -> bool {
        let cached_settings = self.user_settings.get(&recipient_user_id).map(|settings| {
            crate::repository::preferences::UserSettings {
                compactness: settings.compactness,
                theme: settings.theme.clone(),
                notify_mentions: settings.notify_mentions,
                notify_replies: settings.notify_replies,
                notify_moderation: settings.notify_moderation,
            }
        });
        let settings = if let Some(pool) = self.db_pool.clone() {
            block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::repository::preferences::PreferencesRepo
                        .settings_for(&pool, recipient_user_id)
                        .await
                        .ok()
                })
            })
            .or(cached_settings)
            .unwrap_or_default()
        } else {
            cached_settings.unwrap_or_default()
        };
        match kind {
            NotificationKind::Mention => settings.notify_mentions,
            NotificationKind::Reply => settings.notify_replies,
            NotificationKind::PostRemoved
            | NotificationKind::Report
            | NotificationKind::ModAction => settings.notify_moderation,
            NotificationKind::Invite => true,
        }
    }

    pub fn mentioned_user_ids(&self, text: &str) -> Vec<Uuid> {
        let mut ids = Vec::new();
        let mut seen: HashSet<Uuid> = HashSet::new();

        for handle in mentioned_handles(text) {
            if let Some(user_id) = self.user_id_for_handle(&handle) {
                if seen.insert(user_id) {
                    ids.push(user_id);
                }
            }
            if ids.len() >= MAX_MENTIONS_PER_POST {
                break;
            }
        }

        ids
    }

    pub fn notify_mentions_for_content(&mut self, thread_id: Uuid, actor_id: Uuid, body: &str) {
        let message_thread = self.thread_title(&thread_id).unwrap_or_else(|| "thread".to_string());
        let actor_name = self.user_display_name(actor_id);
        let mut already_mentioned: HashSet<Uuid> = HashSet::new();

        for mentioned_user_id in self.mentioned_user_ids(body) {
            if mentioned_user_id == actor_id || !already_mentioned.insert(mentioned_user_id) {
                continue;
            }

            if self.users.get(&mentioned_user_id).map(|user| user.handle.as_str()) == Some("system")
            {
                continue;
            }

            if self.is_user_muted(mentioned_user_id, actor_id) {
                continue;
            }

            self.add_notification(
                mentioned_user_id,
                Some(actor_id),
                NotificationKind::Mention,
                Some(thread_id),
                None,
                Some(format!("{actor_name} mentioned you in '{message_thread}'.")),
            );
        }
    }

    pub fn notify_tag_followers_for_thread(
        &mut self,
        thread_id: Uuid,
        actor_id: Uuid,
        tags: &[String],
    ) {
        if tags.is_empty() {
            return;
        }

        let followed_tags = tags.iter().map(String::as_str).collect::<HashSet<_>>();
        let recipients = self
            .tag_subscriptions
            .iter()
            .filter_map(|(user_id, subscriptions)| {
                if *user_id == actor_id || self.is_user_muted(*user_id, actor_id) {
                    return None;
                }
                let tag =
                    subscriptions.iter().find(|tag| followed_tags.contains(tag.as_str()))?.clone();
                Some((*user_id, tag))
            })
            .collect::<Vec<_>>();

        if recipients.is_empty() {
            return;
        }

        let thread_title = self.thread_title(&thread_id).unwrap_or_else(|| "thread".to_string());
        let actor_name = self.user_display_name(actor_id);
        for (recipient_id, tag) in recipients {
            if !self.reply_notifications_enabled_for(recipient_id) {
                continue;
            }
            self.add_notification(
                recipient_id,
                Some(actor_id),
                NotificationKind::Reply,
                Some(thread_id),
                None,
                Some(format!(
                    "{actor_name} started a new thread matching #{tag}: '{thread_title}'."
                )),
            );
        }
    }

    pub fn notify_thread_reply(&mut self, thread_id: Uuid, post_id: Uuid, actor_id: Uuid) {
        let Some(thread) = self.threads.get(&thread_id).cloned() else {
            return;
        };
        if thread.user_id == actor_id || self.is_user_muted(thread.user_id, actor_id) {
            return;
        }
        if !self.reply_notifications_enabled_for(thread.user_id) {
            return;
        }
        let actor_name = self.user_display_name(actor_id);
        self.add_notification(
            thread.user_id,
            Some(actor_id),
            NotificationKind::Reply,
            Some(thread_id),
            Some(post_id),
            Some(format!("{actor_name} replied to your thread '{title}'.", title = thread.title)),
        );
    }

    pub fn add_notification(
        &mut self,
        recipient_user_id: Uuid,
        actor_user_id: Option<Uuid>,
        kind: NotificationKind,
        thread_id: Option<Uuid>,
        post_id: Option<Uuid>,
        message: Option<String>,
    ) {
        if !self.notifications_enabled_for(recipient_user_id, kind) {
            return;
        }
        let thread_title = match thread_id.and_then(|id| self.threads.get(&id)) {
            Some(thread) => thread.title.clone(),
            None => "General".to_string(),
        };

        let actor_name = actor_user_id
            .map(|id| self.user_display_name(id))
            .unwrap_or_else(|| "system".to_string());

        let final_message = message.unwrap_or_else(|| match kind {
            NotificationKind::Mention => format!("{actor_name} mentioned you."),
            NotificationKind::Reply => format!("{actor_name} replied."),
            NotificationKind::PostRemoved => format!("{actor_name} removed a post."),
            NotificationKind::Report => format!("{actor_name} reported content."),
            NotificationKind::ModAction => format!("{actor_name} changed thread state."),
            NotificationKind::Invite => format!("{actor_name} invited you."),
        });

        let notification = ForumNotification {
            id: Uuid::new_v4(),
            actor_id: actor_user_id,
            kind,
            thread_id,
            thread_title,
            post_id,
            message: final_message,
            created_at: now(),
            is_read: false,
        };

        self.notifications.entry(recipient_user_id).or_default().push(notification.clone());
        if let Some(pool) = self.db_pool.clone() {
            let inserted = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::insert_notification(&pool, recipient_user_id, &notification).await
                })
            });
            match inserted {
                Ok(()) => {
                    self.persist_runtime_state();
                    return;
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        "notification insert in postgres failed; falling back to full sync"
                    );
                }
            }
        }
        self.persist_state();
    }

    pub fn notifications_for_user(&self, user_id: Uuid) -> Vec<ForumNotification> {
        if let Some(pool) = self.db_pool.clone() {
            let notifications = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::list_notifications_for_user(&pool, user_id).await
                })
            });
            match notifications {
                Ok(notifications) => return notifications,
                Err(error) => {
                    if self.requires_postgres() {
                        return Vec::new();
                    }
                    warn!(error = %error, "notification list in postgres failed; using local cache");
                }
            }
        }

        let mut items = self.notifications.get(&user_id).cloned().unwrap_or_default();
        items.sort_by_key(|item| item.created_at);
        items.reverse();
        items
    }

    pub fn unread_notification_count(&self, user_id: Uuid) -> usize {
        if let Some(pool) = self.db_pool.clone() {
            let count = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::count_unread_notifications(&pool, user_id).await
                })
            });
            match count {
                Ok(count) => return count,
                Err(error) => {
                    if self.requires_postgres() {
                        return 0;
                    }
                    warn!(error = %error, "notification count in postgres failed; using local cache");
                }
            }
        }

        self.notifications
            .get(&user_id)
            .map(|notifications| notifications.iter().filter(|item| !item.is_read).count())
            .unwrap_or(0)
    }

    pub fn mark_notification_read(&mut self, user_id: Uuid, notification_id: Uuid, read: bool) {
        if let Some(notifications) = self.notifications.get_mut(&user_id) {
            for item in notifications.iter_mut() {
                if item.id == notification_id {
                    item.is_read = read;
                    break;
                }
            }
            if let Some(pool) = self.db_pool.clone() {
                let updated = block_in_place(|| {
                    Handle::current().block_on(async move {
                        crate::db::update_notification_read(&pool, user_id, notification_id, read)
                            .await
                    })
                });
                match updated {
                    Ok(true) => {
                        self.persist_runtime_state();
                        return;
                    }
                    Ok(false) => {
                        warn!(
                            "notification read update in postgres missed row; repairing from local state"
                        );
                    }
                    Err(error) => {
                        warn!(
                            error = %error,
                            "notification read update in postgres failed; falling back to full sync"
                        );
                    }
                }
            }
            self.persist_state();
        }
    }

    pub fn remove_notification(&mut self, user_id: Uuid, notification_id: Uuid) -> bool {
        let Some(notifications) = self.notifications.get_mut(&user_id) else {
            return false;
        };

        let original_len = notifications.len();
        notifications.retain(|item| item.id != notification_id);
        let removed = notifications.len() != original_len;
        if removed {
            if let Some(pool) = self.db_pool.clone() {
                let deleted = block_in_place(|| {
                    Handle::current().block_on(async move {
                        crate::db::delete_notification(&pool, user_id, notification_id).await
                    })
                });
                match deleted {
                    Ok(true) => {
                        self.persist_runtime_state();
                        return true;
                    }
                    Ok(false) => {
                        warn!(
                            "notification delete in postgres missed row; repairing from local state"
                        );
                    }
                    Err(error) => {
                        warn!(
                            error = %error,
                            "notification delete in postgres failed; falling back to full sync"
                        );
                    }
                }
            }
            self.persist_state();
        }
        removed
    }

    pub fn mark_all_notifications_read(&mut self, user_id: Uuid) {
        if let Some(notifications) = self.notifications.get_mut(&user_id) {
            for item in notifications.iter_mut() {
                item.is_read = true;
            }
            if let Some(pool) = self.db_pool.clone() {
                let updated = block_in_place(|| {
                    Handle::current().block_on(async move {
                        crate::db::mark_all_notifications_read(&pool, user_id).await
                    })
                });
                match updated {
                    Ok(()) => {
                        self.persist_runtime_state();
                        return;
                    }
                    Err(error) => {
                        warn!(
                            error = %error,
                            "notification mark-all in postgres failed; falling back to full sync"
                        );
                    }
                }
            }
            self.persist_state();
        }
    }
}
