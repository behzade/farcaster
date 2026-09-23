use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EditorChoice {
    #[default]
    Neovim,
    VsCode,
    Zed,
    Helix,
}

impl EditorChoice {
    pub const ALL: [Self; 4] = [Self::Neovim, Self::VsCode, Self::Zed, Self::Helix];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Neovim => "Neovim",
            Self::VsCode => "VS Code",
            Self::Zed => "Zed",
            Self::Helix => "Helix",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Neovim => "neovim",
            Self::VsCode => "vscode",
            Self::Zed => "zed",
            Self::Helix => "helix",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "neovim" => Some(Self::Neovim),
            "vscode" => Some(Self::VsCode),
            "zed" => Some(Self::Zed),
            "helix" => Some(Self::Helix),
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
