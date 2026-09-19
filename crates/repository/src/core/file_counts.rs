use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(super) fn parse(patch: &str) -> BTreeMap<PathBuf, Option<(usize, usize)>> {
    let mut files = BTreeMap::new();
    for section in patch.split("\ndiff --git ") {
        let mut old = None;
        let mut new = None;
        let mut renamed = None;
        let mut in_hunk = false;
        let mut binary = false;
        let mut counts = (0, 0);
        for line in section.lines() {
            if line.starts_with("@@ ") {
                in_hunk = true;
            } else if in_hunk {
                counts.0 += usize::from(line.starts_with('+'));
                counts.1 += usize::from(line.starts_with('-'));
            } else if line.starts_with("GIT binary patch")
                || line.starts_with("Binary files ")
                || line.starts_with("Binary file ")
            {
                binary = true;
            } else if let Some(path) = line.strip_prefix("--- ") {
                old =
                    patch_path(path).and_then(|p| p.strip_prefix("a/").ok().map(Path::to_path_buf));
            } else if let Some(path) = line.strip_prefix("+++ ") {
                new =
                    patch_path(path).and_then(|p| p.strip_prefix("b/").ok().map(Path::to_path_buf));
            } else if let Some(path) = line
                .strip_prefix("rename to ")
                .or_else(|| line.strip_prefix("copy to "))
            {
                renamed = patch_path(path);
            }
        }
        if let Some(path) = new.or(renamed).or(old) {
            files.insert(path, (!binary).then_some(counts));
        }
    }
    files
}

// Git quotes special bytes with C escapes (including octal UTF-8 bytes).
fn patch_path(path: &str) -> Option<PathBuf> {
    let path = path.trim_end_matches('\t');
    if !path.starts_with('"') {
        return Some(PathBuf::from(path));
    }
    let mut bytes = path.strip_prefix('"')?.strip_suffix('"')?.bytes();
    let mut decoded = Vec::new();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'\\' {
            match bytes.next()? {
                b'a' => 7,
                b'b' => 8,
                b't' => b'\t',
                b'n' => b'\n',
                b'v' => 11,
                b'f' => 12,
                b'r' => b'\r',
                b'\\' => b'\\',
                b'"' => b'"',
                first @ b'0'..=b'3' => {
                    let second = bytes.next()?.checked_sub(b'0').filter(|n| *n < 8)?;
                    let third = bytes.next()?.checked_sub(b'0').filter(|n| *n < 8)?;
                    (first - b'0') * 64 + second * 8 + third
                }
                _ => return None,
            }
        } else {
            byte
        });
    }
    String::from_utf8(decoded).ok().map(PathBuf::from)
}

#[cfg(test)]
#[path = "file_counts_tests.rs"]
mod tests;
