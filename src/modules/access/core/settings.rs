pub(crate) trait NetworkSettingsStore {
    fn load_proxy(&self) -> Result<Option<String>, String>;
}

pub(crate) fn load_proxy(store: &impl NetworkSettingsStore) -> Result<Option<String>, String> {
    store.load_proxy()
}
