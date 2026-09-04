/// Shorten only a complete, quoted shell wrapper. The expanded row retains the
/// original invocation; unfamiliar forms stay visible rather than being guessed.
pub(super) fn command_summary(command: &str) -> String {
    let command = command.trim();
    let summary = unwrap_shell(command).unwrap_or_else(|| command.to_owned());
    let first = summary.lines().next().unwrap_or_default().trim();
    let mut chars = first.chars();
    let mut preview = chars.by_ref().take(96).collect::<String>();
    if chars.next().is_some() || summary.lines().count() > 1 {
        preview.push('…');
    }
    preview
}

fn unwrap_shell(command: &str) -> Option<String> {
    let (shell, rest) = command.split_once(char::is_whitespace)?;
    let name = shell.rsplit('/').next()?;
    if !matches!(name, "bash" | "sh" | "zsh" | "dash" | "ksh" | "fish") {
        return None;
    }
    let (flags, payload) = rest.trim_start().split_once(char::is_whitespace)?;
    if !matches!(flags, "-c" | "-lc" | "-ic" | "-ilc") {
        return None;
    }
    let mut chars = payload.trim().chars();
    let quote = chars.next()?;
    if !matches!(quote, '\'' | '"') {
        return None;
    }
    let mut result = String::new();
    while let Some(ch) = chars.next() {
        if ch == quote {
            return chars.as_str().trim().is_empty().then_some(result);
        }
        if quote == '"' && ch == '\\' {
            let escaped = chars.next()?;
            if !matches!(escaped, '$' | '`' | '"' | '\\' | '\n') {
                result.push('\\');
            }
            if escaped != '\n' {
                result.push(escaped);
            }
        } else {
            result.push(ch);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_paths_do_not_hide_the_command() {
        assert_eq!(
            command_summary("/nix/store/hash-bash-5.3/bin/bash -lc 'cargo check --offline'"),
            "cargo check --offline"
        );
        assert_eq!(
            command_summary(r#"/bin/zsh -lc "rg \"hello\" src""#),
            r#"rg "hello" src"#
        );
        assert_eq!(
            command_summary("bash -lc 'git status\ngit diff'"),
            "git status…"
        );
    }

    #[test]
    fn unfamiliar_or_compound_wrappers_stay_visible() {
        for command in [
            "env FOO=bar bash -lc 'echo hi'",
            "bash -lc 'echo hi' && deploy",
            "bash -lc 'echo hi' argument",
            "bash -lc 'unfinished",
            "bash -lc echo hi",
            "notbash -lc 'echo hi'",
            "bash --noprofile -c 'echo hi'",
            "echo 'bash -lc something'",
        ] {
            assert_eq!(command_summary(command), command);
        }
    }

    #[test]
    fn truncation_is_visible_and_unicode_safe() {
        assert_eq!(
            command_summary(&"界".repeat(97)),
            format!("{}…", "界".repeat(96))
        );
        assert_eq!(command_summary(""), "");
        assert_eq!(command_summary("cargo check"), "cargo check");
    }
}
