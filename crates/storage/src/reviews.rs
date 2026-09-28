use super::*;

impl StateStore {
    pub fn resolve_caller_session(
        &self,
        caller: &crate::agents::CallerContext,
    ) -> Result<SessionSummary, String> {
        let id = find_caller_session_id(&self.connection, caller)?
            .ok_or("authenticated session is not indexed yet; retry after session discovery")?;
        self.cached_sessions("")?
            .into_iter()
            .find(|session| session.app_session_id == id)
            .ok_or_else(|| "authenticated session has no indexed locator yet".into())
    }

    pub fn register_caller_session(
        &self,
        caller: &crate::agents::CallerContext,
    ) -> Result<i64, String> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
            .map_err(|e| format!("register caller session: {e}"))?;
        let id = register_caller_session_in(&tx, &self.image_directory, caller)?;
        tx.commit()
            .map_err(|e| format!("commit caller session: {e}"))?;
        Ok(id)
    }

    pub fn register_execution_for_caller(
        &self,
        caller: &crate::agents::CallerContext,
        execution: &crate::agents::ExecutionBinding,
    ) -> Result<i64, String> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
            .map_err(|e| format!("register execution turn: {e}"))?;
        let session_id = register_caller_session_in(&tx, &self.image_directory, caller)?;
        tx.execute(
            "INSERT INTO session_turns(id,session_id,prompt_id,started_ms) VALUES(?1,?2,?3,?4)",
            params![execution.turn_id, session_id, execution.prompt_id, now_ms()],
        )
        .map_err(|e| format!("register execution turn: {e}"))?;
        tx.commit()
            .map_err(|e| format!("commit execution turn: {e}"))?;
        Ok(session_id)
    }

    pub fn save_review(
        &self,
        caller: &crate::agents::CallerContext,
        execution: &crate::agents::ExecutionBinding,
        artifact: &serde_json::Value,
    ) -> Result<(), String> {
        let id = artifact
            .pointer("/farcaster_review/id")
            .and_then(serde_json::Value::as_str)
            .ok_or("review is missing its identity")?;
        let changed = self
            .connection
            .execute(
                "INSERT INTO session_reviews(id,session_id,turn_id,artifact,created_ms)
            SELECT ?1,s.id,t.id,?3,?4 FROM sessions s JOIN projects p ON p.id=s.project_id
              JOIN session_turns t ON t.session_id=s.id AND t.id=?2
            WHERE s.harness=?5 AND p.path=?6
              AND ((?7 IS NOT NULL AND s.locator=?7)
                OR (?7 IS NULL AND (s.backend_id=?8 OR s.locator=?9)))",
                params![
                    id,
                    execution.turn_id,
                    artifact.to_string(),
                    now_ms(),
                    caller.backend.as_str(),
                    crate::sessions::normalize_session_path(&caller.project).to_string_lossy(),
                    caller.session_locator.as_ref().map(|path| {
                        crate::sessions::normalize_session_path(path)
                            .to_string_lossy()
                            .into_owned()
                    }),
                    caller.session,
                    crate::sessions::normalize_session_path(Path::new(&caller.session))
                        .to_string_lossy()
                ],
            )
            .map_err(|e| format!("save review submission: {e}"))?;
        if changed != 1 {
            return Err("review execution no longer belongs to the registered session".into());
        }
        Ok(())
    }
}

fn find_caller_session_id(
    connection: &Connection,
    caller: &crate::agents::CallerContext,
) -> Result<Option<i64>, String> {
    let project = crate::sessions::normalize_session_path(&caller.project);
    let locator = caller
        .session_locator
        .as_deref()
        .or_else(|| {
            Path::new(&caller.session)
                .is_absolute()
                .then(|| Path::new(&caller.session))
        })
        .map(crate::sessions::normalize_session_path);
    if let Some(path) = &caller.session_locator
        && crate::agents::external_session_identity(path)
            != Some((caller.backend, caller.session.clone()))
    {
        return Err("caller session locator does not match its native identity".into());
    }
    let effective_profile = match locator.as_deref() {
        Some(path) => super::identity::resolve_session_profile(
            connection,
            path,
            caller.harness_profile_id.as_deref(),
        )?,
        None => caller.harness_profile_id.clone(),
    };
    let mut statement = connection.prepare(
        "SELECT s.id,s.backend_id,s.locator,s.profile_id FROM sessions s JOIN projects p ON p.id=s.project_id WHERE p.path=?1 AND s.harness=?2"
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![project.to_string_lossy().as_ref(), caller.backend.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let candidates = rows
        .iter()
        .filter(|(_, native, path, profile)| {
            let path = path
                .as_deref()
                .map(Path::new)
                .map(crate::sessions::normalize_session_path);
            let native = native.clone().or_else(|| {
                path.as_deref()
                    .and_then(crate::agents::external_session_identity)
                    .map(|(_, id)| id)
            });
            (native.as_deref() == Some(caller.session.as_str())
                || (Path::new(&caller.session).is_absolute() && path == locator))
                && locator
                    .as_ref()
                    .is_none_or(|locator| path.as_ref() == Some(locator))
                && *profile == effective_profile
        })
        .collect::<Vec<_>>();
    let selected = if let Some(hint) = caller.app_session_id {
        let live: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
                [hint.get()],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if live {
            Some(
                *candidates
                    .iter()
                    .find(|row| row.0 == hint.get())
                    .ok_or("caller application identity conflicts with its session scope")?,
            )
        } else {
            if locator.is_none() {
                return Err(
                    "stale caller application identity requires an exact session locator".into(),
                );
            }
            None
        }
    } else {
        None
    };
    let selected = match (selected, candidates.as_slice()) {
        (Some(row), _) => Some(row),
        (None, []) => None,
        (None, [row]) => Some(*row),
        _ => {
            return Err(
                "session ID is ambiguous across indexed sessions; cannot safely resolve caller"
                    .into(),
            );
        }
    };
    if let Some((id, _, _, profile)) = selected {
        if locator.is_none() && caller.harness_profile_id.is_none() && profile.is_some() {
            return Err("profiled caller requires an explicit profile or session locator".into());
        }
        Ok(Some(*id))
    } else {
        Ok(None)
    }
}

fn register_caller_session_in(
    tx: &Transaction<'_>,
    image_directory: &Path,
    caller: &crate::agents::CallerContext,
) -> Result<i64, String> {
    if let Some(locator) = &caller.session_locator
        && crate::agents::external_session_identity(locator)
            != Some((caller.backend, caller.session.clone()))
    {
        return Err("caller session locator does not match its native identity".into());
    }
    if let Some(id) = find_caller_session_id(tx, caller)? {
        return Ok(id);
    }
    let project = ensure_project(tx, &caller.project, u64_to_i64(now_ms()))?;
    let root = image_directory
        .parent()
        .ok_or("session database has no parent")?
        .join("session-locators");
    if caller.session_locator.is_none() && !Path::new(&caller.session).is_absolute() {
        let encoded =
            url::form_urlencoded::byte_serialize(caller.session.as_bytes()).collect::<String>();
        let legacy = crate::sessions::normalize_session_path(
            &root.join(caller.backend.as_str()).join(encoded),
        );
        tx.execute("UPDATE sessions SET backend_id=?1 WHERE harness=?2 AND project_id=?3 AND locator=?4 AND backend_id IS NULL",
            params![caller.session,caller.backend.as_str(),project,legacy.to_string_lossy()]).map_err(|e| format!("bind native session identity: {e}"))?;
    }
    let root = super::identity::family_locator_root(
        &crate::sessions::normalize_session_path(&root),
        &caller.project,
    );
    let identity = caller
        .session_locator
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| caller.session.clone());
    let identity = if !Path::new(&identity).is_absolute()
        && let Some(profile) = &caller.harness_profile_id
    {
        let encoded = url::form_urlencoded::byte_serialize(identity.as_bytes()).collect::<String>();
        root.join("profiles")
            .join(profile)
            .join(caller.backend.as_str())
            .join(encoded)
            .to_string_lossy()
            .into_owned()
    } else {
        identity
    };
    let locator = crate::sessions::normalize_session_path(Path::new(&identity));
    let profile = super::identity::resolve_session_profile(
        tx,
        &locator,
        caller.harness_profile_id.as_deref(),
    )?;
    let id = ensure_locator_session(tx, caller.backend, &identity, project, &root)?;
    if caller.harness_profile_id.is_some() {
        tx.execute(
            "UPDATE sessions SET profile_id=?2 WHERE id=?1",
            params![id, profile],
        )
        .map_err(|error| format!("persist caller launch profile: {error}"))?;
    }
    if !Path::new(&caller.session).is_absolute() {
        let native: Option<String> = tx
            .query_row("SELECT backend_id FROM sessions WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .map_err(|error| error.to_string())?;
        if native
            .as_ref()
            .is_some_and(|native| native != &caller.session)
        {
            return Err("caller native ID conflicts with its indexed locator".into());
        }
        tx.execute(
            "UPDATE sessions SET backend_id=?2 WHERE id=?1",
            params![id, caller.session],
        )
        .map_err(|error| format!("persist caller native ID: {error}"))?;
    }
    validate_caller_app_hint(tx, caller, id)?;
    Ok(id)
}

fn validate_caller_app_hint(
    tx: &Transaction<'_>,
    caller: &crate::agents::CallerContext,
    resolved: i64,
) -> Result<(), String> {
    if let Some(hint) = caller.app_session_id
        && hint.get() != resolved
    {
        let live: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
                [hint.get()],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if live {
            return Err("caller application identity conflicts with its resolved session".into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "reviews_tests.rs"]
mod tests;
