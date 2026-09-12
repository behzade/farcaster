use super::*;

#[cfg(test)]
fn stored_family_identity(
    locator_root: &Path,
    project: &Path,
    harness: &str,
    locator: Option<String>,
    backend_id: Option<String>,
) -> String {
    let Some(locator) = locator else {
        return backend_id.unwrap_or_default();
    };
    let Some(backend_id) = backend_id else {
        return locator;
    };
    let encoded = url::form_urlencoded::byte_serialize(backend_id.as_bytes()).collect::<String>();
    let legacy_synthetic = locator_root.join(harness).join(&encoded);
    let scoped_synthetic = super::identity::family_locator_root(locator_root, project)
        .join(harness)
        .join(encoded);
    let locator = crate::sessions::normalize_session_path(Path::new(&locator));
    if locator == crate::sessions::normalize_session_path(&legacy_synthetic)
        || locator == crate::sessions::normalize_session_path(&scoped_synthetic)
    {
        backend_id
    } else {
        locator.to_string_lossy().into_owned()
    }
}

impl StateStore {
    pub(crate) fn load_expand_transcript_folders(&self) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT value FROM meta WHERE key='expand_transcript_folders'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map(|value| value.as_deref() == Some("true"))
            .map_err(|error| format!("load transcript folder setting: {error}"))
    }

    pub(crate) fn save_expand_transcript_folders(&self, expanded: bool) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES('expand_transcript_folders', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [if expanded { "true" } else { "false" }],
            )
            .map(|_| ())
            .map_err(|error| format!("save transcript folder setting: {error}"))
    }

    pub(crate) fn load_preferred_harness(&self, project: &Path) -> Result<Option<String>, String> {
        // Before the first saved choice, infer it from this project's main sessions.
        let normalized_project = crate::sessions::normalize_session_path(project);
        let legacy_project = project.to_string_lossy();
        self.connection
            .query_row(
                "SELECT COALESCE(
                    (SELECT value FROM meta WHERE key='preferred_harness'),
                    (SELECT harness FROM sessions
                     WHERE submitted=1 AND client_key IS NOT NULL
                       AND project_id IN (SELECT id FROM projects WHERE path IN (?1, ?2))
                       AND parent_id IS NULL AND parent_backend_id IS NULL
                     ORDER BY created_ms DESC, id DESC LIMIT 1))",
                [
                    normalized_project.to_string_lossy().as_ref(),
                    legacy_project.as_ref(),
                ],
                |row| row.get(0),
            )
            .map_err(|error| format!("load preferred harness: {error}"))
            .and_then(|harness: Option<String>| match harness {
                Some(harness) if harness.trim().is_empty() => {
                    Err("The saved backend is empty. Choose a backend.".into())
                }
                harness => Ok(harness),
            })
    }

    pub(crate) fn save_preferred_harness(&self, harness: &str) -> Result<(), String> {
        if harness.trim().is_empty() {
            return Err("Choose a backend before saving the preference.".into());
        }
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES('preferred_harness', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [harness],
            )
            .map(|_| ())
            .map_err(|error| format!("save preferred harness: {error}"))
    }

    pub(crate) fn save_worker_family(
        &self,
        link: &crate::agents::WorkerFamilyLink,
    ) -> Result<(), String> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let project_id = ensure_project(&transaction, &link.project, u64_to_i64(now_ms()))?;
        let locator_root = self
            .image_directory
            .parent()
            .ok_or("state image directory has no parent")?
            .join("session-locators");
        let locator_root = super::identity::family_locator_root(&locator_root, &link.project);
        let parent_id = ensure_locator_session(
            &transaction,
            &link.parent_backend,
            &link.parent_session,
            project_id,
            &locator_root,
        )?;
        let child_id = ensure_locator_session(
            &transaction,
            &link.child_backend,
            &link.child_session,
            project_id,
            &locator_root,
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

    #[cfg(test)]
    pub(crate) fn load_worker_families(
        &self,
    ) -> Result<Vec<crate::agents::WorkerFamilyLink>, String> {
        let locator_root = self
            .image_directory
            .parent()
            .ok_or("state image directory has no parent")?
            .join("session-locators");
        let mut statement = self
            .connection
            .prepare(
                "SELECT child.harness, child.locator, child.backend_id,
                        parent.harness, parent.locator, parent.backend_id,
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
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .map(|row| {
                let (
                    child_backend,
                    child_locator,
                    child_backend_id,
                    parent_backend,
                    parent_locator,
                    parent_backend_id,
                    project,
                    execution,
                ) = row.map_err(|error| error.to_string())?;
                let execution = execution
                    .map(|value| serde_json::from_str(&value))
                    .transpose()
                    .map_err(|error| error.to_string())?
                    .flatten();
                Ok(crate::agents::WorkerFamilyLink {
                    project: crate::sessions::normalize_session_path(Path::new(&project)),
                    child_session: stored_family_identity(
                        &locator_root,
                        Path::new(&project),
                        &child_backend,
                        child_locator,
                        child_backend_id,
                    ),
                    child_backend,
                    parent_session: stored_family_identity(
                        &locator_root,
                        Path::new(&project),
                        &parent_backend,
                        parent_locator,
                        parent_backend_id,
                    ),
                    parent_backend,
                    execution,
                })
            })
            .collect()
    }

    pub(crate) fn load_worker_profiles(&self) -> Result<crate::agents::WorkerProfiles, String> {
        let tasks = self
            .load_json_setting("worker_tasks_json", "worker profiles")?
            .map(crate::agents::WorkerProfiles::from_saved)
            .transpose()?
            .unwrap_or_default();
        tasks.validate()?;
        Ok(tasks)
    }

    pub(crate) fn save_worker_profiles(
        &self,
        tasks: &crate::agents::WorkerProfiles,
    ) -> Result<(), String> {
        tasks.validate()?;
        self.save_json_setting("worker_tasks_json", "worker profiles", tasks)
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

    pub(crate) fn load_configuration_catalogs(
        &self,
    ) -> Result<Vec<CachedConfigurationCatalog>, String> {
        self.load_json_setting("configuration_catalogs_json", "configuration catalogs")
            .map(Option::unwrap_or_default)
            .map(normalize_configuration_catalogs)
    }

    pub(crate) fn save_configuration_catalogs(
        &self,
        catalogs: &[CachedConfigurationCatalog],
    ) -> Result<(), String> {
        self.save_json_setting(
            "configuration_catalogs_json",
            "configuration catalogs",
            &normalize_configuration_catalogs(catalogs.to_vec()),
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
                  WHERE repository_backend IS NOT NULL
                  ORDER BY deleted_at IS NOT NULL, added_ms, path",
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
            preferences
                .entry(crate::sessions::normalize_session_path(Path::new(&path)))
                .or_insert(backend);
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
            let project_id = ensure_project(&transaction, project, u64_to_i64(now_ms()))?;
            transaction
                .execute(
                    "UPDATE projects SET repository_backend=?2 WHERE id=?1",
                    params![project_id, backend],
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

fn normalize_configuration_catalogs(
    catalogs: Vec<CachedConfigurationCatalog>,
) -> Vec<CachedConfigurationCatalog> {
    let mut indexes = BTreeMap::new();
    let mut normalized = Vec::with_capacity(catalogs.len());
    for mut catalog in catalogs {
        catalog.project = crate::sessions::normalize_session_path(&catalog.project);
        let key = (catalog.harness.clone(), catalog.project.clone());
        if let Some(index) = indexes.get(&key) {
            normalized[*index] = catalog;
        } else {
            indexes.insert(key, normalized.len());
            normalized.push(catalog);
        }
    }
    normalized
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

impl StateStore {
    pub(crate) fn load_session_folders(
        &self,
    ) -> Result<crate::app::session_folders::SessionFolders, String> {
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='session_folders'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("load session folders: {error}"))?;
        json.map(|json| {
            serde_json::from_str(&json).map_err(|error| format!("decode session folders: {error}"))
        })
        .transpose()
        .map(Option::unwrap_or_default)
    }

    pub(crate) fn save_session_folders(
        &self,
        folders: &crate::app::session_folders::SessionFolders,
    ) -> Result<(), String> {
        let json = serde_json::to_string(folders)
            .map_err(|error| format!("encode session folders: {error}"))?;
        self.connection.execute(
            "INSERT INTO meta(key,value) VALUES('session_folders',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json]
        ).map_err(|error| format!("save session folders: {error}"))?;
        Ok(())
    }
}
