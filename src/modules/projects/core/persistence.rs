use super::super::contract::Registry;

pub(crate) trait ProjectStore {
    fn allocate_session_id(&mut self, draft_id: &str, created_ms: u64) -> Result<i64, String>;
    fn load_registry(&self) -> Result<Registry, String>;
    fn save_registry(&mut self, registry: &Registry) -> Result<(), String>;
}

pub(crate) fn allocate_session_id(
    store: &mut impl ProjectStore,
    draft_id: &str,
    created_ms: u64,
) -> Result<i64, String> {
    store.allocate_session_id(draft_id, created_ms)
}

pub(crate) fn load_registry(store: &impl ProjectStore) -> Result<Registry, String> {
    store.load_registry()
}

pub(crate) fn save_registry(
    store: &mut impl ProjectStore,
    registry: &Registry,
) -> Result<(), String> {
    store.save_registry(registry)
}
