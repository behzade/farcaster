use std::borrow::Cow;

pub(super) fn display(command: &str) -> Cow<'_, str> {
    script(command).map_or(Cow::Borrowed(command), Cow::Owned)
}

fn script(command: &str) -> Option<String> {
    // Avoid scanning or allocating unbounded scripts just to draw a label.
    if command.len() > 64 * 1024 || !literal_words(command) {
        return None;
    }
    let words = shell_words::split(command).ok()?;
    let [shell, flags, script] = words.as_slice() else {
        return None;
    };
    let shell = shell.rsplit('/').next()?;
    if matches!(shell, "sh" | "bash" | "zsh" | "dash" | "ksh")
        && matches!(flags.as_str(), "-c" | "-lc" | "-cl")
        && !script.trim().is_empty()
    {
        Some(script.clone())
    } else {
        None
    }
}

// shell_words removes quotes but does not evaluate shell syntax. Only unwrap
// literal argv: expansions, operators, comments and extra arguments stay visible.
fn literal_words(command: &str) -> bool {
    let mut quote = None;
    let mut chars = command.chars();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), _) => {}
            (None, '\'' | '"') => quote = Some(ch),
            (_, '\\') => {
                if matches!(chars.next(), None | Some('\n' | '\r')) {
                    return false;
                }
            }
            (_, '$' | '`') => return false,
            (
                None,
                '\n' | '\r' | ';' | '|' | '&' | '<' | '>' | '(' | ')' | '{' | '}' | '*' | '?' | '['
                | ']' | '~' | '#' | '!',
            ) => return false,
            _ => {}
        }
    }
    quote.is_none()
}

#[cfg(test)]
#[path = "command_display_tests.rs"]
mod tests;
