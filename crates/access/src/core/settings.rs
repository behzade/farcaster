pub trait NetworkSettingsStore {
    fn load_proxy(&self) -> Result<Option<String>, String>;
}

pub fn load_proxy(store: &impl NetworkSettingsStore) -> Result<Option<String>, String> {
    store.load_proxy()
}
