use std::{
    ffi::{OsStr, OsString},
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
    Micro,
    Emacs,
    Nano,
    Custom,
}

impl EditorChoice {
    pub const ALL: [Self; 9] = [
        Self::Neovim,
        Self::Vim,
        Self::VsCode,
        Self::Zed,
        Self::Helix,
        Self::Micro,
        Self::Emacs,
        Self::Nano,
        Self::Custom,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Neovim => "Neovim",
            Self::VsCode => "VS Code",
            Self::Zed => "Zed",
            Self::Helix => "Helix",
            Self::Vim => "Vim",
            Self::Micro => "Micro",
            Self::Emacs => "Emacs",
            Self::Nano => "Nano",
            Self::Custom => "Custom",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Neovim => "neovim",
            Self::VsCode => "vscode",
            Self::Zed => "zed",
            Self::Helix => "helix",
            Self::Vim => "vim",
            Self::Micro => "micro",
            Self::Emacs => "emacs",
            Self::Nano => "nano",
            Self::Custom => "custom",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "neovim" => Some(Self::Neovim),
            "vscode" => Some(Self::VsCode),
            "zed" => Some(Self::Zed),
            "helix" => Some(Self::Helix),
            "vim" => Some(Self::Vim),
            "micro" => Some(Self::Micro),
            "emacs" => Some(Self::Emacs),
            "nano" => Some(Self::Nano),
            "custom" => Some(Self::Custom),
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
            Self::Micro => PathBuf::from("micro"),
            Self::Emacs => PathBuf::from("emacs"),
            Self::Nano => PathBuf::from("nano"),
            Self::Custom => PathBuf::new(),
        }
    }

    pub fn available(self, project: &Path, search_path: Option<&OsStr>) -> bool {
        executable_available(&self.program(project, search_path), project, search_path)
    }

    pub fn terminal_command(
        self,
        custom: &str,
        project: &Path,
        search_path: Option<&OsStr>,
    ) -> Result<EditorCommand, String> {
        if self == Self::Custom {
            return EditorCommand::parse(custom);
        }
        Ok(EditorCommand {
            program: self.program(project, search_path),
            arguments: if self == Self::Emacs {
                vec!["-nw".into()]
            } else {
                Vec::new()
            },
        })
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct EditorCommand {
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
}

impl EditorCommand {
    pub fn parse(command: &str) -> Result<Self, String> {
        let parts = shell_words::split(command)
            .map_err(|error| format!("Invalid editor command: {error}"))?;
        let mut parts = parts.into_iter();
        let program = parts
            .next()
            .filter(|program| !program.is_empty())
            .ok_or("Enter an editor command, for example micro -softwrap true.")?;
        if program.contains('\0') {
            return Err("The editor command contains a null character.".into());
        }
        let arguments: Vec<OsString> = parts.map(OsString::from).collect();
        if arguments
            .iter()
            .any(|argument| argument.as_encoded_bytes().contains(&0))
        {
            return Err("The editor command contains a null character.".into());
        }
        Ok(Self {
            program: program.into(),
            arguments,
        })
    }

    pub fn choice(&self) -> EditorChoice {
        match self.program.file_name().and_then(OsStr::to_str) {
            Some("nvim" | "vim" | "vi") => EditorChoice::Vim,
            Some("hx" | "helix") => EditorChoice::Helix,
            Some("micro") => EditorChoice::Micro,
            Some("emacs") => EditorChoice::Emacs,
            Some("nano") => EditorChoice::Nano,
            _ => EditorChoice::Custom,
        }
    }

    pub fn project_arguments(&self, project: &Path) -> Vec<OsString> {
        if self.choice() == EditorChoice::Vim {
            vec![project.as_os_str().to_owned()]
        } else {
            Vec::new()
        }
    }

    pub fn file_arguments(&self, path: &Path, line: Option<u64>) -> Vec<OsString> {
        let mut arguments = Vec::new();
        match self.choice() {
            EditorChoice::Helix => {
                let mut location = path.as_os_str().to_owned();
                if let Some(line) = line {
                    location.push(format!(":{}", line.max(1)));
                }
                arguments.push(location);
            }
            choice => {
                if matches!(
                    choice,
                    EditorChoice::Vim
                        | EditorChoice::Micro
                        | EditorChoice::Emacs
                        | EditorChoice::Nano
                ) && let Some(line) = line
                {
                    arguments.push(format!("+{}", line.max(1)).into());
                }
                arguments.push(path.as_os_str().to_owned());
            }
        }
        arguments
    }

    pub fn review_arguments(&self, locations: &[(PathBuf, Option<u64>)]) -> Vec<OsString> {
        if self.choice() == EditorChoice::Micro && locations.iter().any(|(_, line)| line.is_some())
        {
            let mut arguments = vec!["-parsecursor".into(), "true".into()];
            arguments.extend(locations.iter().map(|(path, line)| {
                let mut location = path.as_os_str().to_owned();
                location.push(format!(":{}:1", line.unwrap_or(1).max(1)));
                location
            }));
            arguments
        } else {
            locations
                .iter()
                .flat_map(|(path, line)| self.file_arguments(path, *line))
                .collect()
        }
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

#[cfg(test)]
#[path = "editors_tests.rs"]
mod tests;
