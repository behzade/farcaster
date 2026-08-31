use std::{collections::BTreeMap, path::PathBuf};

use super::super::BackendPreference;

pub(crate) trait PreferenceStore {
    fn load(&self) -> Result<BTreeMap<PathBuf, BackendPreference>, String>;
    fn save(&self, preferences: &BTreeMap<PathBuf, BackendPreference>) -> Result<(), String>;
}

pub(crate) fn load(
    store: &impl PreferenceStore,
) -> Result<BTreeMap<PathBuf, BackendPreference>, String> {
    store.load()
}

pub(crate) fn save(
    store: &impl PreferenceStore,
    preferences: &BTreeMap<PathBuf, BackendPreference>,
) -> Result<(), String> {
    store.save(preferences)
}
