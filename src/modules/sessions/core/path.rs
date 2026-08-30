use std::path::{Path, PathBuf};

use path_clean::PathClean as _;

pub(crate) fn normalize_lexical(path: &Path) -> PathBuf {
    if path.as_os_str().is_empty() {
        PathBuf::new()
    } else {
        path.clean()
    }
}
