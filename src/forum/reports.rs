use super::*;

impl ForumStore {
    pub fn report_by_id(&self, report_id: Uuid) -> Option<ReportRecord> {
        self.reports.iter().find(|report| report.id == report_id).cloned()
    }

    pub fn reports_queue(&self) -> Vec<ReportRecord> {
        if let Some(pool) = self.db_pool.clone() {
            let reports = block_in_place(|| {
                Handle::current().block_on(async move { crate::db::list_reports(&pool).await })
            });
            match reports {
                Ok(reports) => return reports,
                Err(error) => {
                    if self.requires_postgres() {
                        return Vec::new();
                    }
                    warn!(error = %error, "reports queue in postgres failed; using local cache");
                }
            }
        }

        let mut reports = self.reports.clone();
        reports.sort_by_key(|item| item.created_at);
        reports.reverse();
        reports
    }

    pub fn resolve_report(&mut self, report_id: Uuid, resolved: bool) -> bool {
        for report in self.reports.iter_mut() {
            if report.id == report_id {
                report.resolved = resolved;
                if let Some(pool) = self.db_pool.clone() {
                    let updated = block_in_place(|| {
                        Handle::current().block_on(async move {
                            crate::db::update_report_resolved(&pool, report_id, resolved).await
                        })
                    });
                    match updated {
                        Ok(true) => {
                            self.persist_runtime_state();
                            return true;
                        }
                        Ok(false) => {
                            warn!(
                                "report resolve in postgres missed row; repairing from local state"
                            );
                        }
                        Err(error) => {
                            warn!(error = %error, "report resolve in postgres failed; falling back to full sync");
                        }
                    }
                }
                self.persist_state();
                return true;
            }
        }
        false
    }

    pub fn report_post(
        &mut self,
        reporter_id: Uuid,
        post_id: Option<Uuid>,
        thread_id: Option<Uuid>,
        reason: String,
        details: Option<String>,
    ) -> Result<ReportRecord, ForumActionError> {
        let report = ReportRecord {
            id: Uuid::new_v4(),
            actor_id: reporter_id,
            target_post_id: post_id,
            target_thread_id: thread_id,
            reason: reason.chars().take(120).collect(),
            details,
            created_at: now(),
            resolved: false,
        };
        if let Some(pool) = self.db_pool.clone() {
            let report_for_db = report.clone();
            let inserted = block_in_place(|| {
                Handle::current()
                    .block_on(async move { crate::db::insert_report(&pool, &report_for_db).await })
            });
            match inserted {
                Ok(()) => {
                    self.reports.push(report.clone());
                    self.persist_runtime_state();
                    return Ok(report);
                }
                Err(error) => {
                    if self.requires_postgres() {
                        return Err(self
                            .storage_error(format!("report insert in postgres failed: {error}")));
                    }
                    warn!(error = %error, "report insert in postgres failed; falling back to full sync");
                }
            }
        }
        self.reports.push(report.clone());
        self.persist_state();
        Ok(report)
    }
}
