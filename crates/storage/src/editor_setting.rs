use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EditorChoice {
    #[default]
    Neovim,
    VsCode,
}

impl EditorChoice {
    fn as_str(self) -> &'static str {
        match self {
            Self::Neovim => "neovim",
            Self::VsCode => "vscode",
        }
    }
}

impl StateStore {
    pub fn load_editor_choice(&self) -> Result<EditorChoice, String> {
        let value = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='editor_choice'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("load editor setting: {error}"))?;
        match value.as_deref() {
            None | Some("neovim") => Ok(EditorChoice::Neovim),
            Some("vscode") => Ok(EditorChoice::VsCode),
            Some(value) => Err(format!("unknown saved editor: {value}")),
        }
    }

    pub fn save_editor_choice(&self, choice: EditorChoice) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES('editor_choice', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [choice.as_str()],
            )
            .map(|_| ())
            .map_err(|error| format!("save editor setting: {error}"))
    }
}

#[cfg(test)]
#[path = "editor_setting_tests.rs"]
mod tests;
