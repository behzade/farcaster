use super::*;

impl StateStore {
    pub(crate) fn load_draft_inspector(&self) -> Result<bool, String> {
        self.load_json_meta("draft_inspector", "new-session inspector")
            .map(Option::unwrap_or_default)
    }

    pub(crate) fn save_draft_inspector(&self, visible: bool) -> Result<(), String> {
        self.save_json_meta("draft_inspector", "new-session inspector", &visible)
    }

    pub(crate) fn load_window_placement(&self) -> Result<Option<WindowPlacement>, String> {
        let stored = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='window_placement'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load window placement: {error}"))?;
        stored
            .map(|value| {
                serde_json::from_str(&value)
                    .map_err(|error| format!("decode window placement: {error}"))
            })
            .transpose()
    }

    pub(crate) fn save_window_placement(&self, placement: &WindowPlacement) -> Result<(), String> {
        let value = serde_json::to_string(placement)
            .map_err(|error| format!("encode window placement: {error}"))?;
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES('window_placement', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [value],
            )
            .map_err(|error| format!("save window placement: {error}"))?;
        Ok(())
    }

    pub(crate) fn load_app_session_order(&self) -> Result<Vec<i64>, String> {
        let stored = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='app_session_order'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load application session order: {error}"))?;
        stored.map_or_else(
            || Ok(Vec::new()),
            |value| {
                serde_json::from_str(&value)
                    .map_err(|error| format!("decode application session order: {error}"))
            },
        )
    }

    pub(crate) fn save_app_session_order(&self, order: &[i64]) -> Result<(), String> {
        let value = serde_json::to_string(order)
            .map_err(|error| format!("encode application session order: {error}"))?;
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES('app_session_order', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [value],
            )
            .map(|_| ())
            .map_err(|error| format!("save application session order: {error}"))
    }

    pub(crate) fn load_network_proxy(&self) -> Result<Option<String>, String> {
        self.connection
            .query_row(
                "SELECT value FROM meta WHERE key=?1",
                [NETWORK_PROXY_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load network proxy: {error}"))
    }

    pub(crate) fn save_network_proxy(&self, proxy: Option<&str>) -> Result<(), String> {
        if let Some(proxy) = proxy {
            crate::access::validate_app_proxy(proxy)?;
            self.connection
                .execute(
                    "INSERT INTO meta(key, value) VALUES(?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![NETWORK_PROXY_KEY, proxy],
                )
                .map(|_| ())
                .map_err(|error| format!("save network proxy: {error}"))
        } else {
            self.connection
                .execute("DELETE FROM meta WHERE key=?1", [NETWORK_PROXY_KEY])
                .map(|_| ())
                .map_err(|error| format!("clear network proxy: {error}"))
        }
    }

    pub(crate) fn load_application_modifier(&self) -> Result<Option<String>, String> {
        self.connection
            .query_row(
                "SELECT value FROM meta WHERE key=?1",
                [APPLICATION_MODIFIER_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load application modifier: {error}"))
    }

    pub(crate) fn load_builtin_mcp_enabled(&self) -> Result<bool, String> {
        let value = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key=?1",
                [BUILTIN_MCP_ENABLED_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load built-in MCP setting: {error}"))?;
        match value.as_deref() {
            None | Some("true") => Ok(true),
            Some("false") => Ok(false),
            Some(value) => Err(format!("invalid built-in MCP setting: {value}")),
        }
    }

    pub(crate) fn save_builtin_mcp_enabled(&self, enabled: bool) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES(?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![BUILTIN_MCP_ENABLED_KEY, enabled.to_string()],
            )
            .map(|_| ())
            .map_err(|error| format!("save built-in MCP setting: {error}"))
    }

    pub(crate) fn save_application_settings(
        &self,
        modifier: &str,
        proxy: Option<&str>,
    ) -> Result<(), String> {
        if let Some(proxy) = proxy {
            crate::access::validate_app_proxy(proxy)?;
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|error| format!("start application settings save: {error}"))?;
        let save = |key, value, subject| {
            transaction
                .execute(
                    "INSERT INTO meta(key, value) VALUES(?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![key, value],
                )
                .map(|_| ())
                .map_err(|error| format!("save {subject}: {error}"))
        };
        save(APPLICATION_MODIFIER_KEY, modifier, "application modifier")?;
        if let Some(proxy) = proxy {
            save(NETWORK_PROXY_KEY, proxy, "network proxy")?;
        } else {
            transaction
                .execute("DELETE FROM meta WHERE key=?1", [NETWORK_PROXY_KEY])
                .map_err(|error| format!("clear network proxy: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("commit application settings: {error}"))
    }

    pub(crate) fn load_configuration_catalogs(
        &self,
    ) -> Result<Vec<CachedConfigurationCatalog>, String> {
        self.load_json_meta(CONFIGURATION_CATALOGS_KEY, "configuration catalogs")
            .map(Option::unwrap_or_default)
    }

    pub(crate) fn save_configuration_catalogs(
        &self,
        catalogs: &[CachedConfigurationCatalog],
    ) -> Result<(), String> {
        self.save_json_meta(
            CONFIGURATION_CATALOGS_KEY,
            "configuration catalogs",
            catalogs,
        )
    }

    pub(crate) fn load_session_control_defaults(
        &self,
    ) -> Result<Vec<CachedSessionControlDefaults>, String> {
        self.load_json_meta(SESSION_CONTROL_DEFAULTS_KEY, "session control defaults")
            .map(Option::unwrap_or_default)
    }

    pub(crate) fn save_session_control_defaults(
        &self,
        defaults: &[CachedSessionControlDefaults],
    ) -> Result<(), String> {
        self.save_json_meta(
            SESSION_CONTROL_DEFAULTS_KEY,
            "session control defaults",
            defaults,
        )
    }

    fn load_json_meta<T: DeserializeOwned>(
        &self,
        key: &str,
        subject: &str,
    ) -> Result<Option<T>, String> {
        let stored = self
            .connection
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(|error| format!("load {subject}: {error}"))?;
        stored
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| format!("decode {subject}: {error}"))
            })
            .transpose()
    }

    fn save_json_meta<T: Serialize + ?Sized>(
        &self,
        key: &str,
        subject: &str,
        value: &T,
    ) -> Result<(), String> {
        let value =
            serde_json::to_string(value).map_err(|error| format!("encode {subject}: {error}"))?;
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES(?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )
            .map(|_| ())
            .map_err(|error| format!("save {subject}: {error}"))
    }

    pub(crate) fn load_repository_backend_preferences(
        &self,
    ) -> Result<BTreeMap<PathBuf, String>, String> {
        let stored = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key=?1",
                [REPOSITORY_BACKEND_PREFERENCES_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load repository backend preferences: {error}"))?;
        let preferences = stored.map_or_else(
            || Ok(BTreeMap::new()),
            |value| {
                serde_json::from_str::<BTreeMap<PathBuf, String>>(&value)
                    .map_err(|error| format!("decode repository backend preferences: {error}"))
            },
        )?;
        validate_repository_backend_preferences(&preferences)?;
        Ok(preferences)
    }

    pub(crate) fn save_repository_backend_preferences(
        &self,
        preferences: &BTreeMap<PathBuf, String>,
    ) -> Result<(), String> {
        validate_repository_backend_preferences(preferences)?;
        let value = serde_json::to_string(preferences)
            .map_err(|error| format!("encode repository backend preferences: {error}"))?;
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES(?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![REPOSITORY_BACKEND_PREFERENCES_KEY, value],
            )
            .map(|_| ())
            .map_err(|error| format!("save repository backend preferences: {error}"))
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
