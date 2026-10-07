use gpui::{Context, Task};

use super::FarcasterApp;
use crate::{
    agents::{AgentLaunchConfig, Backend, SignIn, SignInEvent},
    app::RuntimeCommand,
};

#[derive(Default)]
pub(in crate::app) struct HarnessSignInState {
    pub active: Option<SignIn>,
    pub url: Option<String>,
    pub message: Option<String>,
    pub failed: bool,
    task: Option<Task<()>>,
}

impl FarcasterApp {
    pub(in crate::app) fn harness_sign_in_required(
        backend: Backend,
        profile_id: Option<&str>,
    ) -> bool {
        crate::agents::sign_in_required(
            backend,
            &AgentLaunchConfig {
                profile_id: profile_id.map(str::to_owned),
                session_locator_root: crate::app::paths::data_dir()
                    .ok()
                    .map(|root| root.join("session-locators")),
                ..Default::default()
            },
        )
    }

    pub(in crate::app) fn sign_in_harness(
        &mut self,
        backend: Backend,
        profile_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.settings.sign_in.active.is_some() {
            return;
        }
        let result = (|| {
            let config = AgentLaunchConfig {
                profiles: self.settings.harness_profiles.clone(),
                profile_id: profile_id.clone(),
                session_locator_root: Some(crate::app::paths::data_dir()?.join("session-locators")),
                app_proxy: crate::app::persistence::open()?.load_network_proxy()?,
                ..Default::default()
            };
            crate::agents::sign_in(config, backend, self.project.path.clone())
        })();
        self.settings.sign_in.url = None;
        self.settings.sign_in.failed = false;
        match result {
            Err(error) => {
                self.settings.sign_in.message = Some(error);
                self.settings.sign_in.failed = true;
            }
            Ok(sign_in) => {
                let events = sign_in.events.clone();
                self.settings.sign_in.active = Some(sign_in);
                self.settings.sign_in.message = Some(format!(
                    "Starting {} sign-in…",
                    crate::agents::backend_display_name(backend)
                ));
                self.settings.sign_in.task = Some(cx.spawn(async move |weak, cx| {
                    while let Ok(event) = events.recv().await {
                        let finished = matches!(event, SignInEvent::Finished(_));
                        if weak
                            .update(cx, |this, cx| {
                                match event {
                                    SignInEvent::Url(url) => {
                                        if this
                                            .settings
                                            .sign_in
                                            .active
                                            .as_ref()
                                            .is_none_or(|sign_in| sign_in.is_cancelled())
                                        {
                                            return;
                                        }
                                        cx.open_url(&url);
                                        this.settings.sign_in.url = Some(url);
                                        this.settings.sign_in.message =
                                            Some("Finish signing in in your browser.".into());
                                    }
                                    SignInEvent::Finished(result) => {
                                        this.settings.sign_in.active = None;
                                        this.settings.sign_in.url = None;
                                        this.settings.sign_in.failed = result.is_err();
                                        this.settings.sign_in.message = Some(match result {
                                            Ok(()) => {
                                                if this.active_profile_id() == profile_id
                                                    && this.snapshot.harness == Some(backend)
                                                {
                                                    this.send(
                                                        RuntimeCommand::LoadConfiguration {
                                                            harness: backend,
                                                            project: this.project.path.clone(),
                                                        },
                                                        cx,
                                                    );
                                                }
                                                "Signed in. You can start a session.".into()
                                            }
                                            Err(error) => error,
                                        });
                                    }
                                }
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                        if finished {
                            break;
                        }
                    }
                }));
            }
        }
        cx.notify();
    }

    pub(in crate::app) fn cancel_harness_sign_in(&mut self, cx: &mut Context<Self>) {
        if let Some(sign_in) = &self.settings.sign_in.active {
            sign_in.cancel();
            self.settings.sign_in.url = None;
            self.settings.sign_in.message = Some("Cancelling sign-in…".into());
        }
        cx.notify();
    }
}
