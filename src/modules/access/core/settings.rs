pub(crate) trait NetworkSettingsStore {
    fn load_proxy(&self) -> Result<Option<String>, String>;
    fn save_proxy(&self, proxy: Option<&str>) -> Result<(), String>;
}

pub(crate) fn load_proxy(store: &impl NetworkSettingsStore) -> Result<Option<String>, String> {
    store.load_proxy()
}

pub(crate) fn save_proxy(
    store: &impl NetworkSettingsStore,
    proxy: Option<&str>,
) -> Result<(), String> {
    store.save_proxy(proxy)
}
