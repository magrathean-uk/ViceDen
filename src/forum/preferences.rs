use super::*;

impl ForumStore {
    pub fn compactness_for_user(&self, user_id: Uuid) -> i8 {
        self.user_settings.get(&user_id).map(|settings| settings.compactness).unwrap_or_default()
    }

    pub fn theme_for_user(&self, _user_id: Uuid) -> String {
        "dark".to_string()
    }

    pub fn set_theme_for_user(
        &mut self,
        user_id: Uuid,
        _theme: String,
    ) -> Result<(), ForumActionError> {
        let theme = "dark";
        let previous = self.user_settings.get(&user_id).cloned();
        let settings = self.user_settings.entry(user_id).or_default();
        settings.theme = theme.to_string();
        let settings = settings.clone();
        if let Some(pool) = self.db_pool.clone() {
            let updated = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::upsert_user_settings(&pool, user_id, &settings).await
                })
            });
            match updated {
                Ok(()) => {
                    self.persist_runtime_state();
                    return Ok(());
                }
                Err(error) => {
                    if self.requires_postgres() {
                        match previous {
                            Some(previous) => {
                                self.user_settings.insert(user_id, previous);
                            }
                            None => {
                                self.user_settings.remove(&user_id);
                            }
                        }
                        return Err(self.storage_error(format!(
                            "user theme write in postgres failed: {error}"
                        )));
                    }
                    warn!(error = %error, "user theme write in postgres failed; falling back to full sync");
                }
            }
        }
        self.persist_state();
        Ok(())
    }

    pub fn set_compactness_for_user(
        &mut self,
        user_id: Uuid,
        compactness: i8,
    ) -> Result<(), ForumActionError> {
        let compactness = clamp_compactness(compactness);
        let previous = self.user_settings.get(&user_id).cloned();
        let settings = self.user_settings.entry(user_id).or_default();
        settings.compactness = compactness;
        let settings = settings.clone();
        if let Some(pool) = self.db_pool.clone() {
            let updated = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::upsert_user_settings(&pool, user_id, &settings).await
                })
            });
            match updated {
                Ok(()) => {
                    self.persist_runtime_state();
                    return Ok(());
                }
                Err(error) => {
                    if self.requires_postgres() {
                        match previous {
                            Some(previous) => {
                                self.user_settings.insert(user_id, previous);
                            }
                            None => {
                                self.user_settings.remove(&user_id);
                            }
                        }
                        return Err(self.storage_error(format!(
                            "user compactness write in postgres failed: {error}"
                        )));
                    }
                    warn!(
                        error = %error,
                        "user compactness write in postgres failed; falling back to full sync"
                    );
                }
            }
        }
        self.persist_state();
        Ok(())
    }
}
