use super::*;
use crate::agents::Backend;

fn stored_family_identity(
    locator_root: &Path,
    project: &Path,
    harness: Backend,
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
    let legacy_synthetic = locator_root.join(harness.as_str()).join(&encoded);
    let scoped_synthetic = super::identity::family_locator_root(locator_root, project)
        .join(harness.as_str())
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
    pub fn harness_profile_in_use(&self, id: &str) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE profile_id=?1)",
                [id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    }

    pub fn load_harness_profiles(&self) -> Result<Vec<crate::agents::HarnessProfile>, String> {
        let saved: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='harness_profiles'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("load harness profiles: {error}"))?;
        let profiles: Vec<crate::agents::HarnessProfile> = saved
            .map(|saved| serde_json::from_str(&saved))
            .transpose()
            .map_err(|error| format!("decode harness profiles: {error}"))?
            .unwrap_or_default();
        validate_harness_profiles(&profiles)?;
        Ok(profiles)
    }

    pub fn save_harness_profiles(
        &self,
        profiles: &[crate::agents::HarnessProfile],
    ) -> Result<(), String> {
        validate_harness_profiles(profiles)?;
        let saved = serde_json::to_string(profiles)
            .map_err(|error| format!("encode harness profiles: {error}"))?;
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES('harness_profiles', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [saved],
            )
            .map(|_| ())
            .map_err(|error| format!("save harness profiles: {error}"))
    }

    pub fn load_preferred_profile_id(&self) -> Result<Option<String>, String> {
        self.connection
            .query_row(
                "SELECT value FROM meta WHERE key='preferred_harness_profile'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("load preferred harness profile: {error}"))
    }

    pub fn save_preferred_profile_id(&self, id: Option<&str>) -> Result<(), String> {
        match id {
            Some(id) => self.connection.execute(
                "INSERT INTO meta(key,value) VALUES('preferred_harness_profile',?1)
                     ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [id],
            ),
            None => self
                .connection
                .execute("DELETE FROM meta WHERE key='preferred_harness_profile'", []),
        }
        .map(|_| ())
        .map_err(|error| format!("save preferred harness profile: {error}"))
    }

    pub fn load_expand_transcript_folders(&self) -> Result<bool, String> {
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

    pub fn save_expand_transcript_folders(&self, expanded: bool) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES('expand_transcript_folders', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [if expanded { "true" } else { "false" }],
            )
            .map(|_| ())
            .map_err(|error| format!("save transcript folder setting: {error}"))
    }

    pub fn load_preferred_harness(&self, project: &Path) -> Result<Option<Backend>, String> {
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
                Some(harness) => harness.parse().map(Some),
                None => Ok(None),
            })
    }

    pub fn save_preferred_harness(&self, harness: Backend) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES('preferred_harness', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [harness.as_str()],
            )
            .map(|_| ())
            .map_err(|error| format!("save preferred harness: {error}"))
    }

    pub fn save_worker_family(&self, link: &crate::agents::WorkerFamilyLink) -> Result<(), String> {
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
            link.parent_backend,
            &link.parent_session,
            project_id,
            &locator_root,
        )?;
        let child_id = ensure_locator_session(
            &transaction,
            link.child_backend,
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
        let routing = link
            .routing
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        if let Some(routing) = &link.routing {
            transaction
                .execute(
                    "UPDATE worker_families SET routing_json=NULL
                       WHERE child_id != ?1
                         AND child_id IN (SELECT id FROM sessions WHERE parent_id=?2)
                         AND json_valid(routing_json)
                         AND lower(json_extract(routing_json, '$.name'))=lower(?3)",
                    params![child_id, parent_id, routing.name],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction
            .execute(
                "INSERT INTO worker_families(child_id, execution_json, routing_json)
                 VALUES(?1, ?2, ?3)
                 ON CONFLICT(child_id) DO UPDATE SET
                   execution_json=excluded.execution_json,
                   routing_json=excluded.routing_json",
                params![child_id, execution, routing],
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
    pub fn load_worker_families(&self) -> Result<Vec<crate::agents::WorkerFamilyLink>, String> {
        self.load_worker_families_filtered(false)
    }

    pub fn load_worker_routes(&self) -> Result<Vec<crate::agents::WorkerFamilyLink>, String> {
        self.load_worker_families_filtered(true)
    }

    fn load_worker_families_filtered(
        &self,
        active_only: bool,
    ) -> Result<Vec<crate::agents::WorkerFamilyLink>, String> {
        let locator_root = self
            .image_directory
            .parent()
            .ok_or("state image directory has no parent")?
            .join("session-locators");
        let active_filter = if active_only {
            "WHERE p.deleted_at IS NULL
               AND child.archived_at IS NULL
               AND parent.archived_at IS NULL"
        } else {
            ""
        };
        let query = format!(
            "SELECT child.harness, child.locator, child.backend_id,
                        parent.harness, parent.locator, parent.backend_id,
                        p.path, f.execution_json, f.routing_json
                   FROM worker_families f
                   JOIN sessions child ON child.id = f.child_id
                   JOIN sessions parent ON parent.id = child.parent_id
                   JOIN projects p ON p.id = child.project_id
                   {active_filter}"
        );
        let mut statement = self
            .connection
            .prepare(&query)
            .map_err(|error| error.to_string())?;
        statement
            .query_map([], |row| {
                Ok((
                    super::backend::get(row, 0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    super::backend::get(row, 3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
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
                    routing,
                ) = row.map_err(|error| error.to_string())?;
                let execution = execution.and_then(|value| {
                    serde_json::from_str(&value)
                        .map_err(|error| {
                            log::warn!("Ignore malformed worker execution: {error}");
                        })
                        .ok()
                        .flatten()
                });
                let routing: Option<crate::agents::WorkerRouting> = routing.and_then(|value| {
                    serde_json::from_str(&value)
                        .map_err(|error| {
                            log::warn!("Ignore malformed worker routing: {error}");
                        })
                        .ok()
                });
                Ok(crate::agents::WorkerFamilyLink {
                    project: crate::sessions::normalize_session_path(Path::new(&project)),
                    child_session: stored_family_identity(
                        &locator_root,
                        Path::new(&project),
                        child_backend,
                        child_locator,
                        child_backend_id,
                    ),
                    child_backend,
                    parent_session: stored_family_identity(
                        &locator_root,
                        Path::new(&project),
                        parent_backend,
                        parent_locator,
                        parent_backend_id,
                    ),
                    parent_backend,
                    execution,
                    routing,
                })
            })
            .collect()
    }

    pub fn load_worker_profiles(&self) -> Result<crate::agents::WorkerProfiles, String> {
        let tasks = self
            .load_json_setting("worker_tasks_json", "worker profiles")?
            .map(crate::agents::WorkerProfiles::from_saved)
            .transpose()?
            .unwrap_or_default();
        tasks.validate()?;
        Ok(tasks)
    }

    pub fn save_worker_profiles(
        &self,
        tasks: &crate::agents::WorkerProfiles,
    ) -> Result<(), String> {
        tasks.validate()?;
        self.save_json_setting("worker_tasks_json", "worker profiles", tasks)
    }

    pub fn load_window_placement(&self) -> Result<Option<WindowPlacement>, String> {
        self.load_json_setting("window_placement_json", "window placement")
    }

    pub fn save_window_placement(&self, placement: &WindowPlacement) -> Result<(), String> {
        self.save_json_setting("window_placement_json", "window placement", placement)
    }

    pub fn load_app_session_order(&self) -> Result<Vec<i64>, String> {
        Ok(self
            .load_json_setting("app_session_order_json", "application session order")?
            .unwrap_or_default())
    }

    pub fn save_app_session_order(&self, order: &[i64]) -> Result<(), String> {
        self.save_json_setting("app_session_order_json", "application session order", order)
    }

    pub fn load_network_proxy(&self) -> Result<Option<String>, String> {
        self.load_text_setting("network_proxy", "network proxy")
    }

    pub fn save_network_proxy(&self, proxy: Option<&str>) -> Result<(), String> {
        if let Some(proxy) = proxy {
            crate::access::validate_app_proxy(proxy)?;
        }
        self.ensure_ui_state()?;
        self.connection
            .execute("UPDATE ui_state SET network_proxy=?1 WHERE id=1", [proxy])
            .map(|_| ())
            .map_err(|error| format!("save network proxy: {error}"))
    }

    pub fn load_builtin_mcp_enabled(&self) -> Result<bool, String> {
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

    pub fn save_builtin_mcp_enabled(&self, enabled: bool) -> Result<(), String> {
        self.ensure_ui_state()?;
        self.connection
            .execute(
                "UPDATE ui_state SET builtin_mcp_enabled=?1 WHERE id=1",
                [i64::from(enabled)],
            )
            .map(|_| ())
            .map_err(|error| format!("save built-in MCP setting: {error}"))
    }

    pub fn load_configuration_catalogs(&self) -> Result<Vec<CachedConfigurationCatalog>, String> {
        self.load_json_setting("configuration_catalogs_json", "configuration catalogs")
            .map(Option::unwrap_or_default)
            .map(normalize_configuration_catalogs)
    }

    pub fn save_configuration_catalogs(
        &self,
        catalogs: &[CachedConfigurationCatalog],
    ) -> Result<(), String> {
        self.save_json_setting(
            "configuration_catalogs_json",
            "configuration catalogs",
            &normalize_configuration_catalogs(catalogs.to_vec()),
        )
    }

    pub fn load_session_control_defaults(
        &self,
    ) -> Result<Vec<CachedSessionControlDefaults>, String> {
        self.load_json_setting("session_control_defaults_json", "session control defaults")
            .map(Option::unwrap_or_default)
    }

    pub fn save_session_control_defaults(
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

    pub fn load_repository_backend_preferences(&self) -> Result<BTreeMap<PathBuf, String>, String> {
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

    pub fn save_repository_backend_preferences(
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

fn validate_harness_profiles(profiles: &[crate::agents::HarnessProfile]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for profile in profiles {
        profile.validate()?;
        if !ids.insert(&profile.id) {
            return Err(format!("duplicate harness profile ID: {}", profile.id));
        }
        if !names.insert(profile.name.to_lowercase()) {
            return Err(format!("duplicate harness profile name: {}", profile.name));
        }
    }
    Ok(())
}

fn normalize_configuration_catalogs(
    catalogs: Vec<CachedConfigurationCatalog>,
) -> Vec<CachedConfigurationCatalog> {
    let mut indexes = BTreeMap::new();
    let mut normalized = Vec::with_capacity(catalogs.len());
    for mut catalog in catalogs {
        catalog.project = crate::sessions::normalize_session_path(&catalog.project);
        let key = (
            catalog.harness,
            catalog.profile_id.clone(),
            catalog.project.clone(),
        );
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
    pub(crate) fn load_theme_css(&self) -> Result<Option<String>, String> {
        self.load_meta_value("theme_css", "themes")
    }

    pub(crate) fn save_theme_css(&self, css: &str) -> Result<(), String> {
        self.save_meta_value("theme_css", css, "themes")
    }

    pub(crate) fn load_active_theme(&self) -> Result<Option<String>, String> {
        self.load_meta_value("theme_selected", "active theme")
    }

    pub(crate) fn save_active_theme(&self, name: &str) -> Result<(), String> {
        self.save_meta_value("theme_selected", name, "active theme")
    }

    fn load_meta_value(&self, key: &str, subject: &str) -> Result<Option<String>, String> {
        self.connection
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(|error| format!("load {subject}: {error}"))
    }

    fn save_meta_value(&self, key: &str, value: &str, subject: &str) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO meta(key,value) VALUES(?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )
            .map(|_| ())
            .map_err(|error| format!("save {subject}: {error}"))
    }

    pub fn load_session_folders(&self) -> Result<crate::sessions::SessionFolders, String> {
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

    pub fn save_session_folders(
        &self,
        folders: &crate::sessions::SessionFolders,
    ) -> Result<(), String> {
        let json = serde_json::to_string(folders)
            .map_err(|error| format!("encode session folders: {error}"))?;
        self.connection.execute(
            "INSERT INTO meta(key,value) VALUES('session_folders',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json]
        ).map_err(|error| format!("save session folders: {error}"))?;
        Ok(())
    }
}
