use super::*;

impl StateStore {
    pub(crate) fn save_worker_family(
        &self,
        link: &crate::agents::WorkerFamilyLink,
    ) -> Result<(), String> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let project_id = ensure_project(&transaction, &link.project, u64_to_i64(now_ms()))?;
        let parent_id = ensure_locator_session(
            &transaction,
            &link.parent_backend,
            &link.parent_session,
            project_id,
        )?;
        let child_id = ensure_locator_session(
            &transaction,
            &link.child_backend,
            &link.child_session,
            project_id,
        )?;
        transaction
            .execute(
                "UPDATE sessions SET parent_id=?2 WHERE id=?1",
                params![child_id, parent_id],
            )
            .map_err(|error| error.to_string())?;
        let execution =
            serde_json::to_string(&link.execution).map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO worker_families(child_id, execution_json) VALUES(?1, ?2)
                 ON CONFLICT(child_id) DO UPDATE SET execution_json=excluded.execution_json",
                params![child_id, execution],
            )
            .map_err(|error| error.to_string())?;
        if let Some(execution) = &link.execution {
            transaction
                .execute(
                    "INSERT INTO session_models(session_id, provider, model, effort)
                     VALUES(?1, ?2, ?3, ?4)
                     ON CONFLICT(session_id) DO UPDATE SET
                       provider=excluded.provider, model=excluded.model, effort=excluded.effort",
                    params![
                        child_id,
                        execution.provider,
                        execution.model,
                        execution.effort
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    pub(crate) fn load_worker_families(
        &self,
    ) -> Result<Vec<crate::agents::WorkerFamilyLink>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT child.id, child.harness, child.locator, parent.harness, parent.locator,
                        p.path, f.execution_json
                   FROM worker_families f
                   JOIN sessions child ON child.id = f.child_id
                   JOIN sessions parent ON parent.id = child.parent_id
                   JOIN projects p ON p.id = child.project_id",
            )
            .map_err(|error| error.to_string())?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .map(|row| {
                let (
                    child_backend,
                    child_locator,
                    parent_backend,
                    parent_locator,
                    project,
                    execution,
                ) = row.map_err(|error| error.to_string())?;
                let execution = execution
                    .map(|value| serde_json::from_str(&value))
                    .transpose()
                    .map_err(|error| error.to_string())?
                    .flatten();
                Ok(crate::agents::WorkerFamilyLink {
                    project: PathBuf::from(project),
                    child_backend,
                    child_session: child_locator.unwrap_or_default(),
                    parent_backend,
                    parent_session: parent_locator.unwrap_or_default(),
                    execution,
                })
            })
            .collect()
    }

    pub(crate) fn load_worker_tasks(&self) -> Result<crate::agents::WorkerTasks, String> {
        let tasks: crate::agents::WorkerTasks = self
            .load_json_setting("worker_tasks_json", "worker tasks")?
            .unwrap_or_default();
        tasks.validate()?;
        Ok(tasks)
    }

    pub(crate) fn load_window_placement(&self) -> Result<Option<WindowPlacement>, String> {
        self.load_json_setting("window_placement_json", "window placement")
    }

    pub(crate) fn save_window_placement(&self, placement: &WindowPlacement) -> Result<(), String> {
        self.save_json_setting("window_placement_json", "window placement", placement)
    }

    pub(crate) fn load_app_session_order(&self) -> Result<Vec<i64>, String> {
        Ok(self
            .load_json_setting("app_session_order_json", "application session order")?
            .unwrap_or_default())
    }

    pub(crate) fn save_app_session_order(&self, order: &[i64]) -> Result<(), String> {
        self.save_json_setting("app_session_order_json", "application session order", order)
    }

    pub(crate) fn load_network_proxy(&self) -> Result<Option<String>, String> {
        self.load_text_setting("network_proxy", "network proxy")
    }

    pub(crate) fn save_network_proxy(&self, proxy: Option<&str>) -> Result<(), String> {
        if let Some(proxy) = proxy {
            crate::access::validate_app_proxy(proxy)?;
        }
        self.ensure_ui_state()?;
        self.connection
            .execute("UPDATE ui_state SET network_proxy=?1 WHERE id=1", [proxy])
            .map(|_| ())
            .map_err(|error| format!("save network proxy: {error}"))
    }

    pub(crate) fn load_application_modifier(&self) -> Result<Option<String>, String> {
        self.load_text_setting("application_modifier", "application modifier")
    }

    pub(crate) fn load_builtin_mcp_enabled(&self) -> Result<bool, String> {
        let value = self
            .connection
            .query_row(
                "SELECT builtin_mcp_enabled FROM ui_state WHERE id=1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("load built-in MCP setting: {error}"))?;
        Ok(!matches!(value, Some(0)))
    }

    pub(crate) fn save_builtin_mcp_enabled(&self, enabled: bool) -> Result<(), String> {
        self.ensure_ui_state()?;
        self.connection
            .execute(
                "UPDATE ui_state SET builtin_mcp_enabled=?1 WHERE id=1",
                [i64::from(enabled)],
            )
            .map(|_| ())
            .map_err(|error| format!("save built-in MCP setting: {error}"))
    }

    #[cfg(test)]
    pub(crate) fn save_application_settings(
        &self,
        modifier: &str,
        proxy: Option<&str>,
    ) -> Result<(), String> {
        self.save_application_settings_with_workers(modifier, proxy, None)
    }

    pub(crate) fn save_application_settings_with_workers(
        &self,
        modifier: &str,
        proxy: Option<&str>,
        tasks: Option<&crate::agents::WorkerTasks>,
    ) -> Result<(), String> {
        if let Some(tasks) = tasks {
            tasks.validate()?;
        }
        if let Some(proxy) = proxy {
            crate::access::validate_app_proxy(proxy)?;
        }
        let tasks = tasks
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| format!("encode worker tasks: {error}"))?;
        self.ensure_ui_state()?;
        self.connection
            .execute(
                "UPDATE ui_state SET application_modifier=?1, network_proxy=?2,
                   worker_tasks_json=COALESCE(?3, worker_tasks_json) WHERE id=1",
                params![modifier, proxy, tasks],
            )
            .map(|_| ())
            .map_err(|error| format!("save application settings: {error}"))
    }

    pub(crate) fn load_configuration_catalogs(
        &self,
    ) -> Result<Vec<CachedConfigurationCatalog>, String> {
        self.load_json_setting("configuration_catalogs_json", "configuration catalogs")
            .map(Option::unwrap_or_default)
    }

    pub(crate) fn save_configuration_catalogs(
        &self,
        catalogs: &[CachedConfigurationCatalog],
    ) -> Result<(), String> {
        self.save_json_setting(
            "configuration_catalogs_json",
            "configuration catalogs",
            catalogs,
        )
    }

    pub(crate) fn load_session_control_defaults(
        &self,
    ) -> Result<Vec<CachedSessionControlDefaults>, String> {
        self.load_json_setting("session_control_defaults_json", "session control defaults")
            .map(Option::unwrap_or_default)
    }

    pub(crate) fn save_session_control_defaults(
        &self,
        defaults: &[CachedSessionControlDefaults],
    ) -> Result<(), String> {
        self.save_json_setting(
            "session_control_defaults_json",
            "session control defaults",
            defaults,
        )
    }

    fn load_json_setting<T: DeserializeOwned>(
        &self,
        column: &str,
        subject: &str,
    ) -> Result<Option<T>, String> {
        let stored = self.load_text_setting(column, subject)?;
        stored
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| format!("decode {subject}: {error}"))
            })
            .transpose()
    }

    fn save_json_setting<T: Serialize + ?Sized>(
        &self,
        column: &str,
        subject: &str,
        value: &T,
    ) -> Result<(), String> {
        let value =
            serde_json::to_string(value).map_err(|error| format!("encode {subject}: {error}"))?;
        self.ensure_ui_state()?;
        self.connection
            .execute(
                &format!("UPDATE ui_state SET {column}=?1 WHERE id=1"),
                [value],
            )
            .map(|_| ())
            .map_err(|error| format!("save {subject}: {error}"))
    }

    fn load_text_setting(&self, column: &str, subject: &str) -> Result<Option<String>, String> {
        self.connection
            .query_row(
                &format!("SELECT {column} FROM ui_state WHERE id=1"),
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|error| format!("load {subject}: {error}"))
            .map(Option::flatten)
    }

    fn ensure_ui_state(&self) -> Result<(), String> {
        self.connection
            .execute("INSERT OR IGNORE INTO ui_state(id) VALUES(1)", [])
            .map(|_| ())
            .map_err(|error| format!("ensure ui_state: {error}"))
    }

    pub(crate) fn load_repository_backend_preferences(
        &self,
    ) -> Result<BTreeMap<PathBuf, String>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT path, repository_backend FROM projects
                  WHERE repository_backend IS NOT NULL",
            )
            .map_err(|error| format!("load repository backend preferences: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("query repository backend preferences: {error}"))?;
        let mut preferences = BTreeMap::new();
        for row in rows {
            let (path, backend) = row.map_err(|error| error.to_string())?;
            preferences.insert(PathBuf::from(path), backend);
        }
        validate_repository_backend_preferences(&preferences)?;
        Ok(preferences)
    }

    pub(crate) fn save_repository_backend_preferences(
        &self,
        preferences: &BTreeMap<PathBuf, String>,
    ) -> Result<(), String> {
        validate_repository_backend_preferences(preferences)?;
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|error| format!("start repository backend preferences: {error}"))?;
        transaction
            .execute("UPDATE projects SET repository_backend=NULL", [])
            .map_err(|error| format!("clear repository backend preferences: {error}"))?;
        for (project, backend) in preferences {
            transaction
                .execute(
                    "INSERT INTO projects(path, added_ms, repository_backend)
                     VALUES(?1, ?2, ?3)
                     ON CONFLICT(path) DO UPDATE SET repository_backend=excluded.repository_backend",
                    params![project.to_string_lossy(), u64_to_i64(now_ms()), backend],
                )
                .map_err(|error| {
                    format!(
                        "save repository backend preference for {}: {error}",
                        project.display()
                    )
                })?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit repository backend preferences: {error}"))
    }
}

fn validate_repository_backend_preferences(
    preferences: &BTreeMap<PathBuf, String>,
) -> Result<(), String> {
    for (project, backend) in preferences {
        if !project.is_absolute() {
            return Err(format!(
                "repository backend preference project path is not absolute: {}",
                project.display()
            ));
        }
        if !REPOSITORY_BACKENDS.contains(&backend.as_str()) {
            return Err(format!(
                "unknown repository backend preference for {}: {backend}",
                project.display()
            ));
        }
    }
    Ok(())
}
