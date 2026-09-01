#[derive(Clone, Default, Eq, PartialEq)]
pub(crate) struct NetworkConfiguration {
    pub(crate) app_proxy: Option<String>,
}

impl std::fmt::Debug for NetworkConfiguration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NetworkConfiguration")
            .field("app_proxy", &self.app_proxy.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}
