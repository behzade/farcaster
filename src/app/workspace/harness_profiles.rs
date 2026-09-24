use std::path::PathBuf;

use gpui::{Context, Window};

use super::FarcasterApp;
use crate::agents::{Backend, HarnessProfile};

fn entered_path(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(value)
}

impl FarcasterApp {
    pub(in crate::app) fn remove_harness_profile(&mut self, id: String, cx: &mut Context<Self>) {
        let result = (|| {
            let store = crate::app::persistence::open()?;
            if store.harness_profile_in_use(&id)? {
                return Err("This profile is in use by a session. Delete those sessions before removing it.".to_owned());
            }
            let mut profiles = self.settings.harness_profiles.list()?;
            profiles.retain(|profile| profile.id != id);
            store.save_harness_profiles(&profiles)?;
            if store.load_preferred_profile_id()?.as_deref() == Some(&id) {
                store.save_preferred_profile_id(None)?;
                self.sessions.preferred_profile_id = None;
            }
            self.settings.harness_profiles.replace(profiles)
        })();
        self.settings.harness_profile_error = result.err();
        cx.notify();
    }

    pub(in crate::app) fn choose_harness_profile_backend(
        &mut self,
        backend: Backend,
        cx: &mut Context<Self>,
    ) {
        self.settings.harness_profile_backend = backend;
        self.settings.harness_profile_error = None;
        cx.notify();
    }

    pub(in crate::app) fn add_harness_profile(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self
            .settings
            .harness_profile_name
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let executable = entered_path(
            self.settings
                .harness_profile_executable
                .read(cx)
                .value()
                .trim(),
        );
        let directory = self
            .settings
            .harness_profile_data_directory
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let profile = HarnessProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            backend: self.settings.harness_profile_backend,
            executable,
            data_directory: (crate::agents::profile_data_environment_key(
                self.settings.harness_profile_backend,
            )
            .is_some()
                && !directory.is_empty())
            .then(|| entered_path(&directory)),
        };
        let result = (|| {
            profile.validate()?;
            if profile.executable.is_absolute() && !profile.executable.is_file() {
                return Err(format!(
                    "Executable was not found: {}",
                    profile.executable.display()
                ));
            }
            let mut profiles = self.settings.harness_profiles.list()?;
            if profiles
                .iter()
                .any(|known| known.name.eq_ignore_ascii_case(&profile.name))
            {
                return Err(format!(
                    "A harness profile named {} already exists",
                    profile.name
                ));
            }
            profiles.push(profile);
            crate::app::persistence::open()?.save_harness_profiles(&profiles)?;
            self.settings.harness_profiles.replace(profiles)
        })();
        match result {
            Ok(()) => {
                self.settings.harness_profile_error = None;
                self.settings
                    .harness_profile_name
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.settings
                    .harness_profile_executable
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.settings
                    .harness_profile_data_directory
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            Err(error) => self.settings.harness_profile_error = Some(error),
        }
        cx.notify();
    }
}
