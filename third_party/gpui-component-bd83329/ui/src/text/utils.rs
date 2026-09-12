use gpui::{ImageSource, SharedUri};

const NUMBERED_PREFIXES_1: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const NUMBERED_PREFIXES_2: &str = "abcdefghijklmnopqrstuvwxyz";

const BULLETS: [&str; 5] = ["•", "◦", "▪", "‣", "⁃"];

/// Returns the prefix for a list item.
pub(super) fn list_item_prefix(ix: usize, ordered: bool, depth: usize, start: u32) -> String {
    if ordered {
        let number = u64::from(start).saturating_add(ix as u64);
        if depth == 0 || number == 0 {
            return format!("{}. ", number);
        }
        let alphabet_index = number - 1;

        let alphabet = if depth == 1 {
            NUMBERED_PREFIXES_1
        } else {
            NUMBERED_PREFIXES_2
        };
        let letter = alphabet.as_bytes()[(alphabet_index % alphabet.len() as u64) as usize];
        return format!("{}. ", char::from(letter));
    } else {
        let depth = depth.min(BULLETS.len() - 1);
        let bullet = BULLETS[depth];
        return format!("{} ", bullet);
    }
}

/// Converts a document image URL into an [`ImageSource`] without granting
/// implicit filesystem access.
///
/// Document-provided values remain URI-backed, including `file://` and
/// scheme-less strings.
pub(super) fn image_source(url: &SharedUri) -> ImageSource {
    url.clone().into()
}

#[cfg(test)]
#[path = "utils_tests.rs"]
mod tests;
