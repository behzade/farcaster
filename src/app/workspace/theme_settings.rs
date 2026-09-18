use super::*;
use crate::app::ui::theme::{
    Appearance, Theme, ThemeDefinition, ThemeLibrary, ThemeToken, color_hex, editable_tokens,
    install_component_theme, parse_hex, set_active, suggested_file_name,
};

pub(in crate::app) struct ThemeSettings {
    pub(in crate::app) library: ThemeLibrary,
    pub(in crate::app) draft: Option<ThemeDefinition>,
    pub(in crate::app) tokens: Vec<(ThemeToken, Entity<InputState>)>,
    pub(in crate::app) name: Option<Entity<InputState>>,
    pub(in crate::app) error: Option<String>,
    pub(in crate::app) status: Option<String>,
    subscriptions: Vec<Subscription>,
    save: Option<Task<()>>,
    loaded: bool,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            library: ThemeLibrary::default(),
            draft: None,
            tokens: Vec::new(),
            name: None,
            error: None,
            status: None,
            subscriptions: Vec::new(),
            save: None,
            loaded: true,
        }
    }
}

impl ThemeSettings {
    pub(in crate::app) fn load(css: Option<&str>, selected: Option<&str>) -> Self {
        let Some(css) = css else {
            return Self::default();
        };
        match ThemeLibrary::from_css(css, selected) {
            Ok(library) => Self {
                library,
                ..Self::default()
            },
            Err(error) => Self {
                error: Some(error),
                loaded: false,
                ..Self::default()
            },
        }
    }

    pub(in crate::app) fn editable(&self) -> bool {
        self.loaded
    }
}

impl FarcasterApp {
    pub(in crate::app) fn activate_theme(&mut self, cx: &mut Context<Self>) {
        let definition = self.settings.themes.library.selected();
        set_active(
            Theme::from_colors(definition.colors),
            definition.appearance,
            definition.tokens,
        );
        install_component_theme(cx);
        self.notify_appearance(cx);
        cx.refresh_windows();
    }

    pub(in crate::app) fn refresh_theme_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.themes.tokens.clear();
        self.settings.themes.name = None;
        self.settings.themes.subscriptions.clear();
        let Some(draft) = self.settings.themes.draft.clone() else {
            return;
        };
        let name = cx.new(|cx| InputState::new(window, cx).default_value(draft.name.clone()));
        let subscription = cx.subscribe_in(&name, window, |this, state, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                let value = state.read(cx).value().to_string();
                this.commit_theme_rename(value, window, cx);
            }
        });
        self.settings.themes.subscriptions.push(subscription);
        self.settings.themes.name = Some(name);
        for token in editable_tokens() {
            let input = cx
                .new(|cx| InputState::new(window, cx).default_value(color_hex(draft.color(token))));
            let subscription = cx.subscribe_in(&input, window, move |this, state, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = state.read(cx).value().to_owned();
                    this.set_theme_token(token, &value, cx);
                }
            });
            self.settings.themes.subscriptions.push(subscription);
            self.settings.themes.tokens.push((token, input));
        }
    }

    pub(in crate::app) fn select_theme(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.settings.themes.library.select(name) {
            self.settings.themes.error = Some(error);
            cx.notify();
            return;
        }
        self.settings.themes.draft = self
            .settings
            .themes
            .library
            .user_themes()
            .iter()
            .find(|theme| theme.name == name)
            .cloned();
        self.settings.themes.error = None;
        self.activate_theme(cx);
        self.schedule_theme_save(cx);
        self.refresh_theme_editor(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn create_theme_from(
        &mut self,
        source: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(parent) = self.settings.themes.library.find(source) else {
            self.settings.themes.error = Some(format!("Unknown theme: {source}"));
            cx.notify();
            return;
        };
        let name = self
            .settings
            .themes
            .library
            .unique_name(&format!("{} copy", parent.name));
        let draft = ThemeDefinition {
            name: name.clone(),
            ..parent
        };
        if let Err(error) = self.settings.themes.library.upsert(draft.clone()) {
            self.settings.themes.error = Some(error);
            cx.notify();
            return;
        }
        if let Err(error) = self.settings.themes.library.select(&name) {
            self.settings.themes.error = Some(error);
            cx.notify();
            return;
        }
        self.settings.themes.draft = Some(draft);
        self.settings.themes.error = None;
        self.settings.themes.status = Some(format!("{name} is ready to edit."));
        self.activate_theme(cx);
        self.schedule_theme_save(cx);
        self.refresh_theme_editor(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn set_theme_appearance(
        &mut self,
        appearance: Appearance,
        cx: &mut Context<Self>,
    ) {
        let Some(mut draft) = self.settings.themes.draft.clone() else {
            return;
        };
        if draft.appearance == appearance {
            return;
        }
        draft.appearance = appearance;
        self.store_theme_draft(draft, cx);
    }

    pub(in crate::app) fn set_theme_token(
        &mut self,
        token: ThemeToken,
        value: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(color) = parse_hex(value) else {
            return;
        };
        let Some(mut draft) = self.settings.themes.draft.clone() else {
            return;
        };
        if draft.color(token) == color {
            return;
        }
        draft.set_color(token, color);
        self.store_theme_draft(draft, cx);
    }

    pub(in crate::app) fn commit_theme_rename(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = self.settings.themes.draft.clone() else {
            return;
        };
        let trimmed = name.trim();
        if trimmed == draft.name {
            return;
        }
        match self.settings.themes.library.rename(&draft.name, trimmed) {
            Ok(()) => {
                self.settings.themes.draft = Some(ThemeDefinition {
                    name: trimmed.to_owned(),
                    ..draft
                });
                self.settings.themes.error = None;
                self.settings.themes.status = Some(format!("Renamed to {trimmed}."));
                self.schedule_theme_save(cx);
            }
            Err(error) => {
                self.settings.themes.error = Some(error);
                if let Some(input) = self.settings.themes.name.as_ref() {
                    input.update(cx, |input, cx| {
                        input.set_value(draft.name.clone(), window, cx);
                    });
                }
            }
        }
        cx.notify();
    }

    pub(in crate::app) fn delete_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.settings.themes.draft.clone() else {
            return;
        };
        match self.settings.themes.library.remove(&draft.name) {
            Ok(()) => {
                self.settings.themes.draft = None;
                self.settings.themes.error = None;
                self.settings.themes.status = Some(format!("Deleted {}.", draft.name));
                self.activate_theme(cx);
                self.schedule_theme_save(cx);
                self.refresh_theme_editor(window, cx);
            }
            Err(error) => self.settings.themes.error = Some(error),
        }
        cx.notify();
    }

    pub(in crate::app) fn export_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.settings.themes.library.selected_name().to_owned();
        let css = match self.settings.themes.library.export(&name) {
            Ok(css) => css,
            Err(error) => {
                self.settings.themes.error = Some(error);
                cx.notify();
                return;
            }
        };
        let directory =
            crate::app::infrastructure::paths::data_dir().unwrap_or_else(|_| PathBuf::from("."));
        let file_name = suggested_file_name(&name);
        let selected = cx.prompt_for_new_path(&directory, Some(&file_name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = selected.await else {
                return;
            };
            let written = std::fs::write(&path, css);
            let _ = this.update_in(cx, |this, _window, cx| {
                match written {
                    Ok(()) => {
                        this.settings.themes.error = None;
                        this.settings.themes.status =
                            Some(format!("Exported to {}", path.display()));
                    }
                    Err(error) => {
                        this.settings.themes.error = Some(format!("export theme: {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::app) fn import_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import theme CSS".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = selected.await else {
                return;
            };
            let Some(path) = paths.pop() else {
                return;
            };
            let contents = std::fs::read_to_string(&path)
                .map_err(|error| format!("read {}: {error}", path.display()));
            let _ = this.update_in(cx, |this, _window, cx| {
                match contents {
                    Ok(css) => match this.settings.themes.library.import(&css) {
                        Ok(names) => {
                            this.settings.themes.status =
                                Some(format!("Imported {}.", names.join(", ")));
                            this.settings.themes.error = None;
                            if let Err(error) = this.persist_themes() {
                                this.settings.themes.error = Some(error);
                            }
                        }
                        Err(error) => this.settings.themes.error = Some(error),
                    },
                    Err(error) => this.settings.themes.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn store_theme_draft(&mut self, draft: ThemeDefinition, cx: &mut Context<Self>) {
        if let Err(error) = self.settings.themes.library.upsert(draft.clone()) {
            self.settings.themes.error = Some(error);
            cx.notify();
            return;
        }
        self.settings.themes.draft = Some(draft);
        self.settings.themes.error = None;
        self.activate_theme(cx);
        self.schedule_theme_save(cx);
        cx.notify();
    }

    fn persist_themes(&mut self) -> Result<(), String> {
        if !self.settings.themes.editable() {
            return Err("Saved themes could not be read. Restart Farcaster to edit themes.".into());
        }
        let css = self.settings.themes.library.to_css();
        let selected = self.settings.themes.library.selected_name().to_owned();
        let store = crate::app::infrastructure::persistence::StateStore::open()?;
        store.save_theme_css(&css)?;
        store.save_active_theme(&selected)
    }

    fn schedule_theme_save(&mut self, cx: &mut Context<Self>) {
        self.settings.themes.save = Some(cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(400))
                .await;
            let _ = weak.update(cx, |this, cx| {
                this.settings.themes.save = None;
                if let Err(error) = this.persist_themes() {
                    this.settings.themes.error = Some(error);
                }
                cx.notify();
            });
        }));
    }
}

#[cfg(test)]
#[path = "theme_settings_tests.rs"]
mod tests;
