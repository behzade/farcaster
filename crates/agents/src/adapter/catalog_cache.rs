//! Shared request-ready catalogs, keyed by backend, account profile, and project.
use crate::{AgentLaunchConfig, Backend, ConfigurationCatalog};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

type CatalogResult = Result<ConfigurationCatalog, String>;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    harness: Backend,
    profile: Option<String>,
    project: PathBuf,
    program: PathBuf,
    arguments: Vec<String>,
    root: Option<PathBuf>,
    profile_directory: Option<PathBuf>,
}

impl Key {
    fn new(config: &AgentLaunchConfig, harness: Backend, project: &Path) -> Result<Self, String> {
        let config = super::for_backend(harness).launch_configuration(config);
        config.validate_profile_backend(harness)?;
        let profile = config.selected_profile()?;
        Ok(Self {
            harness,
            profile: config.profile_id.clone(),
            project: project.to_owned(),
            program: profile
                .as_ref()
                .map_or(config.program.clone(), |p| p.executable.clone()),
            arguments: config.prefix_args.clone(),
            root: config.locator_root(),
            profile_directory: profile.and_then(|p| p.data_directory),
        })
    }
}

#[derive(Default)]
struct State {
    catalogs: HashMap<Key, ConfigurationCatalog>,
    requests: HashMap<Key, Arc<OnceLock<CatalogResult>>>,
}

#[derive(Default)]
struct Cache {
    state: Mutex<State>,
}

impl Cache {
    fn load(&self, key: Key, fetch: impl FnOnce() -> CatalogResult) -> CatalogResult {
        self.lookup(key, false, fetch)
    }

    fn lookup(
        &self,
        key: Key,
        refresh: bool,
        fetch: impl FnOnce() -> CatalogResult,
    ) -> CatalogResult {
        let request = {
            let mut state = self.state.lock().expect("catalog cache");
            if !refresh && let Some(catalog) = state.catalogs.get(&key) {
                return Ok(catalog.clone());
            }
            state.requests.entry(key.clone()).or_default().clone()
        };
        // Share an in-flight fetch. A background refresh does not block readers
        // of the last complete catalog, and its failure does not erase that catalog.
        let result = request.get_or_init(fetch).clone();
        let mut state = self.state.lock().expect("catalog cache");
        if state
            .requests
            .get(&key)
            .is_some_and(|current| Arc::ptr_eq(current, &request))
        {
            state.requests.remove(&key);
            if let Ok(catalog) = &result {
                state.catalogs.insert(key, catalog.clone());
            }
        }
        result
    }

    fn seed(&self, key: Key, catalog: ConfigurationCatalog) {
        self.state
            .lock()
            .expect("catalog cache")
            .catalogs
            .entry(key)
            .or_insert(catalog);
    }
}

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Cache::default)
}

pub(super) fn load(
    config: &AgentLaunchConfig,
    harness: Backend,
    project: &Path,
    fetch: impl FnOnce() -> CatalogResult,
) -> CatalogResult {
    cache().load(Key::new(config, harness, project)?, fetch)
}

/// Hydrate the shared lookup from the central persisted catalog. Old display-only
/// catalogs remain useful to the UI but must be refreshed before building requests.
pub fn seed_configuration_catalog(
    config: &AgentLaunchConfig,
    harness: Backend,
    project: &Path,
    catalog: &ConfigurationCatalog,
) {
    if super::for_backend(harness).catalog_is_complete(catalog)
        && let Ok(key) = Key::new(config, harness, project)
    {
        cache().seed(key, catalog.clone());
    }
}

pub fn refresh_configuration_catalog(
    config: &AgentLaunchConfig,
    harness: Backend,
    project: &Path,
) -> CatalogResult {
    cache().lookup(Key::new(config, harness, project)?, true, || {
        super::for_backend(harness).configuration_catalog(config, project)
    })
}

pub(super) fn invalidate(config: &AgentLaunchConfig, harness: Backend) {
    let mut state = cache().state.lock().expect("catalog cache");
    state
        .catalogs
        .retain(|key, _| key.harness != harness || key.profile != config.profile_id);
    state
        .requests
        .retain(|key, _| key.harness != harness || key.profile != config.profile_id);
}

#[cfg(test)]
#[path = "catalog_cache_tests.rs"]
mod tests;
