mod nono;

use super::{process::PiRpcProcess, wire::PiCommand};
use crate::agents::HarnessAccessMode;

pub(super) trait PiSandboxAdapter: Sync {
    fn id(&self) -> &'static str;
    fn control_command<'a>(&self, commands: &'a [PiCommand]) -> Result<Option<&'a str>, String>;
    fn access_modes(&self) -> &'static [HarnessAccessMode];
    fn launch_mode(&self, requested: HarnessAccessMode) -> Result<HarnessAccessMode, String> {
        let modes = self.access_modes();
        let selected = if requested == HarnessAccessMode::Auto {
            modes.first().copied()
        } else {
            Some(requested)
        };
        selected
            .filter(|mode| modes.contains(mode))
            .ok_or_else(|| format!("{} does not support the requested sandbox mode", self.id()))
    }
    fn confirm(
        &self,
        process: &mut PiRpcProcess,
        control: &str,
        mode: HarnessAccessMode,
    ) -> Result<(), String>;
    fn mode_report(
        &self,
        request: &crate::agents::extensions::ExtensionUiRequest,
    ) -> Option<Result<HarnessAccessMode, String>>;
}

const ADAPTERS: &[&dyn PiSandboxAdapter] = &[&nono::Nono];

pub(super) fn discover(
    commands: &[PiCommand],
) -> Result<Option<(&'static dyn PiSandboxAdapter, &str)>, String> {
    let mut detected = None;
    for &adapter in ADAPTERS {
        if let Some(control) = adapter.control_command(commands)? {
            if detected.is_some() {
                return Err("Multiple Pi sandbox adapters detected".into());
            }
            detected = Some((adapter, control));
        }
    }
    Ok(detected)
}

pub(in crate::modules::agents::adapter) fn access_modes(
    id: Option<&str>,
) -> &'static [HarnessAccessMode] {
    // Auto is only an internal launch preference until discovery completes.
    let Some(id) = id else {
        return &[HarnessAccessMode::Auto];
    };
    ADAPTERS
        .iter()
        .find(|adapter| adapter.id() == id)
        .map_or(&[], |adapter| adapter.access_modes())
}
