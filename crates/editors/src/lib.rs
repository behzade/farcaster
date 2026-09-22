use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum EditorChoice {
    #[default]
    Neovim,
    VsCode,
    Zed,
    Helix,
    Vim,
}

impl EditorChoice {
    pub const ALL: [Self; 5] = [
        Self::Neovim,
        Self::Vim,
        Self::VsCode,
        Self::Zed,
        Self::Helix,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Neovim => "Neovim",
            Self::VsCode => "VS Code",
            Self::Zed => "Zed",
            Self::Helix => "Helix",
            Self::Vim => "Vim",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Neovim => "neovim",
            Self::VsCode => "vscode",
            Self::Zed => "zed",
            Self::Helix => "helix",
            Self::Vim => "vim",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "neovim" => Some(Self::Neovim),
            "vscode" => Some(Self::VsCode),
            "zed" => Some(Self::Zed),
            "helix" => Some(Self::Helix),
            "vim" => Some(Self::Vim),
            _ => None,
        }
    }

    pub fn program(self, project: &Path, search_path: Option<&OsStr>) -> PathBuf {
        match self {
            Self::Neovim => std::env::var_os("FARCASTER_NVIM")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("nvim")),
            Self::VsCode => PathBuf::from("code"),
            Self::Zed if executable_available(Path::new("zed"), project, search_path) => {
                PathBuf::from("zed")
            }
            Self::Zed => PathBuf::from("zeditor"),
            Self::Helix => PathBuf::from("hx"),
            Self::Vim => PathBuf::from("vim"),
        }
    }

    pub fn available(self, project: &Path, search_path: Option<&OsStr>) -> bool {
        executable_available(&self.program(project, search_path), project, search_path)
    }
}

pub fn executable_available(program: &Path, project: &Path, search_path: Option<&OsStr>) -> bool {
    if program.is_absolute() {
        return is_executable(program);
    }
    if program.components().count() > 1 {
        return is_executable(&project.join(program));
    }
    search_path.is_some_and(|path| {
        std::env::split_paths(path).any(|directory| {
            let directory = if directory.is_absolute() {
                directory
            } else {
                project.join(directory)
            };
            is_executable(&directory.join(program))
        })
    })
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

use farcaster_agents::{program_available, program_available_in};

pub struct TextEditor {
    pub id: &'static str,
    pub name: &'static str,
    pub program: &'static str,
    pub icon: EditorIcon,
    pub line_argument: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorIcon {
    Neovim,
    Vim,
    Helix,
    Micro,
    Emacs,
    Nano,
    Generic,
}

pub const TEXT_EDITORS: [TextEditor; 8] = [
    TextEditor {
        id: "nvim",
        name: "Neovim",
        program: "nvim",
        icon: EditorIcon::Neovim,
        line_argument: true,
    },
    TextEditor {
        id: "vim",
        name: "Vim",
        program: "vim",
        icon: EditorIcon::Vim,
        line_argument: true,
    },
    TextEditor {
        id: "helix",
        name: "Helix",
        program: "hx",
        icon: EditorIcon::Helix,
        line_argument: false,
    },
    TextEditor {
        id: "micro",
        name: "Micro",
        program: "micro",
        icon: EditorIcon::Micro,
        line_argument: true,
    },
    TextEditor {
        id: "vi",
        name: "vi",
        program: "vi",
        icon: EditorIcon::Vim,
        line_argument: true,
    },
    TextEditor {
        id: "kakoune",
        name: "Kakoune",
        program: "kak",
        icon: EditorIcon::Generic,
        line_argument: false,
    },
    TextEditor {
        id: "emacs",
        name: "Emacs",
        program: "emacs",
        icon: EditorIcon::Emacs,
        line_argument: true,
    },
    TextEditor {
        id: "nano",
        name: "nano",
        program: "nano",
        icon: EditorIcon::Nano,
        line_argument: true,
    },
];

pub struct TextEditorStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub program: String,
    pub icon: EditorIcon,
    pub available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorCommand {
    pub program: String,
    pub arguments: Vec<String>,
}

impl EditorCommand {
    pub fn parse(command: &str) -> Result<Self, String> {
        let mut parts = split_command(command)?;
        if parts.is_empty() {
            return Err("The editor command is empty.".into());
        }
        let program = parts.remove(0);
        Ok(Self {
            program,
            arguments: parts,
        })
    }

    pub fn of(editor: &TextEditor) -> Self {
        Self {
            program: editor_program(editor),
            arguments: Vec::new(),
        }
    }

    pub fn command_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.arguments.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn name(&self) -> String {
        let program = self.program_name();
        text_editor(&program).map_or(program, |editor| editor.name.to_owned())
    }

    pub fn icon(&self) -> EditorIcon {
        text_editor(&self.program_name()).map_or(EditorIcon::Generic, |editor| editor.icon)
    }

    pub fn is_neovim(&self) -> bool {
        self.program_name() == "nvim" || self.program == nvim_program()
    }

    pub fn supports_line_argument(&self) -> bool {
        text_editor(&self.program_name()).is_some_and(|editor| editor.line_argument)
    }

    pub fn available(&self) -> bool {
        program_available(Path::new(&self.program))
    }

    fn program_name(&self) -> String {
        Path::new(&self.program).file_name().map_or_else(
            || self.program.clone(),
            |name| name.to_string_lossy().into_owned(),
        )
    }
}

pub fn text_editor(id: &str) -> Option<&'static TextEditor> {
    TEXT_EDITORS
        .iter()
        .find(|editor| editor.id == id || editor.program == id)
}

pub fn text_editor_statuses() -> Vec<TextEditorStatus> {
    text_editor_statuses_in(std::env::var_os("PATH").as_deref())
}

fn text_editor_statuses_in(search_path: Option<&OsStr>) -> Vec<TextEditorStatus> {
    TEXT_EDITORS
        .iter()
        .map(|editor| TextEditorStatus {
            id: editor.id,
            name: editor.name,
            program: editor_program(editor),
            icon: editor.icon,
            available: editor_available(editor, search_path),
        })
        .collect()
}

pub fn installed_text_editor() -> Option<&'static TextEditor> {
    installed_text_editor_in(std::env::var_os("PATH").as_deref())
}

fn installed_text_editor_in(search_path: Option<&OsStr>) -> Option<&'static TextEditor> {
    TEXT_EDITORS
        .iter()
        .find(|editor| editor_available(editor, search_path))
}

fn editor_available(editor: &TextEditor, search_path: Option<&OsStr>) -> bool {
    program_available_in(Path::new(&editor_program(editor)), search_path)
}

fn editor_program(editor: &TextEditor) -> String {
    if editor.id == "nvim" {
        return nvim_program();
    }
    editor.program.to_owned()
}

fn nvim_program() -> String {
    std::env::var("FARCASTER_NVIM")
        .or_else(|_| std::env::var("GPUI_NVIM"))
        .ok()
        .filter(|program| !program.trim().is_empty())
        .unwrap_or_else(|| "nvim".to_owned())
}

/// The Neovim executable a test run should exercise.
pub fn neovim_executable() -> std::path::PathBuf {
    std::path::PathBuf::from(nvim_program())
}

pub fn resolve_text_editor(saved: Option<&str>) -> Result<EditorCommand, String> {
    match saved.map(str::trim).filter(|saved| !saved.is_empty()) {
        Some(saved) => EditorCommand::parse(saved),
        None => installed_text_editor()
            .map(EditorCommand::of)
            .ok_or_else(|| {
                "No text editor found. Install one or set a command in Settings.".to_owned()
            }),
    }
}

fn split_command(command: &str) -> Result<Vec<String>, String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut characters = command.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\'' | '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if !quoted => {
                match characters.next() {
                    Some(escaped) => {
                        current.push(escaped);
                        started = true;
                    }
                    None => current.push(character),
                };
            }
            character if character.is_whitespace() && !quoted => {
                if started {
                    parts.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            character => {
                current.push(character);
                started = true;
            }
        }
    }
    if quoted {
        return Err("The editor command has an unclosed quote.".into());
    }
    if started {
        parts.push(current);
    }
    Ok(parts)
}

#[cfg(test)]
#[path = "editors_tests.rs"]
mod tests;
