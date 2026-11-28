use super::*;
use agents::WorkerStatus;
use sessions::activity::AgentOutcome;

fn status_value(status: WorkerStatus) -> &'static str {
    match status {
        WorkerStatus::Idle => "done",
        WorkerStatus::Failed => "failed",
        WorkerStatus::Stopped => "stopped",
        WorkerStatus::Running | WorkerStatus::Pending | WorkerStatus::NeedsInput => "active",
    }
}

pub(super) fn migrate(tx: &Transaction<'_>) -> Result<(), String> {
    let has_status = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('worker_families') WHERE name='last_status')",
        [],
        |row| row.get::<_, bool>(0),
    ).map_err(|error| error.to_string())?;
    if has_status {
        return Ok(());
    }
    tx.execute_batch(
        "ALTER TABLE worker_families ADD COLUMN last_status TEXT
         CHECK(last_status IN ('done','failed','stopped','active'))",
    )
    .map_err(|e| e.to_string())?;
    let profiles = super::settings::read_harness_profiles(tx)?;
    let mut stmt = tx
        .prepare(
            "SELECT s.id, s.harness, s.locator, s.profile_id
         FROM worker_families f JOIN sessions s ON s.id=f.child_id
         WHERE s.locator IS NOT NULL",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                super::backend::get(row, 1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (id, harness, locator, profile_id) = row.map_err(|e| e.to_string())?;
        let path = Path::new(&locator);
        if let Some(status) = super::settings::read_profile_history(
            &profiles,
            harness,
            path,
            profile_id,
            agents::historical_worker_status,
        ) {
            tx.execute(
                "UPDATE worker_families SET last_status=?2 WHERE child_id=?1",
                params![id, status_value(status)],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

impl StateStore {
    pub fn save_worker_status(
        &self,
        id: sessions::AppSessionId,
        status: WorkerStatus,
    ) -> Result<(), String> {
        let changed = self
            .connection
            .execute(
                "UPDATE worker_families SET last_status=?2 WHERE child_id=?1",
                params![id.get(), status_value(status)],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(format!("No worker family for session {}", id.get()));
        }
        Ok(())
    }

    pub fn worker_outcomes(&self) -> Result<BTreeMap<i64, Option<AgentOutcome>>, String> {
        let mut stmt = self
            .connection
            .prepare("SELECT child_id,last_status FROM worker_families")
            .map_err(|e| e.to_string())?;
        stmt.query_map([], |row| {
            let status = match row.get::<_, Option<String>>(1)?.as_deref() {
                Some("done") => Some(AgentOutcome::Complete),
                Some("failed") => Some(AgentOutcome::Failed),
                Some("active" | "stopped") => Some(AgentOutcome::Incomplete),
                _ => None,
            };
            Ok((row.get(0)?, status))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
#[path = "worker_status_tests.rs"]
mod tests;
