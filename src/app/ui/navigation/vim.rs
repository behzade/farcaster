//! Stateful Vim motion input. App activation and its keys never pass through here.
use super::shortcuts::keyboard_command;
use crate::app::views::transcript::list::KeyboardCommand;

#[derive(Default)]
pub(crate) struct VimInput {
    prefix: Option<char>,
    search: Option<(String, bool)>,
}

#[derive(Debug, PartialEq)]
pub(super) enum Input {
    Pass,
    Pending,
    Command(KeyboardCommand),
}

impl VimInput {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn hint(&self) -> Option<String> {
        if let Some((text, forward)) = &self.search {
            Some(format!("{}{text}", if *forward { '/' } else { '?' }))
        } else if let Some(prefix) = self.prefix {
            Some(prefix.to_string())
        } else {
            None
        }
    }

    pub(super) fn key(&mut self, key: &str, modifiers: gpui::Modifiers, held: bool) -> Input {
        use KeyboardCommand::*;
        if key == "escape" && !modifiers.modified() {
            let searching = self.search.is_some();
            self.clear();
            return Input::Command(if searching { SearchCancel } else { Cancel });
        }
        let plain = super::shortcuts::plain_key(key, modifiers);
        if let Some((text, forward)) = self.search.as_mut() {
            let Some(key) = plain else {
                return Input::Pending;
            };
            match key.as_str() {
                "enter" => {
                    self.search = None;
                    return Input::Command(SearchAccept);
                }
                "backspace" => {
                    use unicode_segmentation::UnicodeSegmentation as _;
                    if let Some((offset, _)) = text.grapheme_indices(true).last() {
                        text.truncate(offset);
                    }
                    return Input::Command(SearchBackspace);
                }
                _ => {
                    let character = if key == "space" {
                        Some(' ')
                    } else {
                        let mut chars = key.chars();
                        chars.next().filter(|_| chars.next().is_none())
                    };
                    if let Some(character) = character {
                        text.push(character);
                        let _ = forward;
                        return Input::Command(SearchChar(character));
                    }
                    return Input::Pending;
                }
            }
        }
        if held
            && (self.prefix.is_some()
                || plain
                    .as_deref()
                    .is_some_and(|k| matches!(k, "g" | "z" | "f" | "F" | "t" | "T" | "/" | "?")))
        {
            return Input::Pending;
        }
        if let Some(prefix) = self.prefix.take() {
            let Some(key) = plain else {
                return Input::Pending;
            };
            if prefix == 'z' {
                return match key.as_str() {
                    "t" => Input::Command(Align(0)),
                    "z" => Input::Command(Align(1)),
                    "b" => Input::Command(Align(2)),
                    _ => Input::Pending,
                };
            }
            if prefix == 'g' {
                let command = match key.as_str() {
                    "g" => Start,
                    "e" => PreviousWordEnd(false),
                    "E" => PreviousWordEnd(true),
                    "j" => ScreenDown,
                    "k" => ScreenUp,
                    "0" => ScreenStart,
                    "^" => ScreenFirst,
                    "$" => ScreenEnd,
                    "_" => LastNonblank,
                    _ => return Input::Pending,
                };
                return Input::Command(command);
            }
            let mut chars = key.chars();
            let character = if key == "space" {
                Some(' ')
            } else {
                chars.next().filter(|_| chars.next().is_none())
            };
            return character
                .map(|character| {
                    Input::Command(Find {
                        character,
                        forward: matches!(prefix, 'f' | 't'),
                        till: matches!(prefix, 't' | 'T'),
                    })
                })
                .unwrap_or(Input::Pending);
        }
        if let Some(key) = plain.as_deref() {
            if key.len() == 1 && matches!(key.as_bytes()[0], b'1'..=b'9') {
                return Input::Pass;
            }
            if matches!(key, "g" | "z" | "f" | "F" | "t" | "T") {
                self.prefix = key.chars().next();
                return Input::Pending;
            }
            if matches!(key, "/" | "?") {
                let forward = key == "/";
                self.search = Some((String::new(), forward));
                return Input::Command(SearchStart(forward));
            }
        }
        let command = keyboard_command(key, modifiers, None);
        match command {
            Some(command) if !held || command.repeats() => Input::Command(command),
            Some(_) => Input::Pending,
            None => Input::Pass,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(input: &mut VimInput, value: &str) -> Input {
        let stroke = gpui::Keystroke::parse(value).unwrap();
        input.key(&stroke.key, stroke.modifiers, false)
    }
    #[test]
    fn digits_remain_sessions_except_zero_and_find_targets() {
        let mut input = VimInput::default();
        for number in 1..=9 {
            assert_eq!(key(&mut input, &number.to_string()), Input::Pass);
        }
        assert_eq!(
            key(&mut input, "0"),
            Input::Command(KeyboardCommand::LineStart)
        );
        assert_eq!(
            key(&mut input, "e"),
            Input::Command(KeyboardCommand::WordEnd)
        );
        assert_eq!(key(&mut input, "t"), Input::Pending);
        assert_eq!(
            key(&mut input, "2"),
            Input::Command(KeyboardCommand::Find {
                character: '2',
                forward: true,
                till: true
            })
        );
        key(&mut input, "g");
        assert_eq!(
            key(&mut input, "e"),
            Input::Command(KeyboardCommand::PreviousWordEnd(false))
        );
        key(&mut input, "f");
        key(&mut input, "escape");
        assert_eq!(
            key(&mut input, "b"),
            Input::Command(KeyboardCommand::WordBackward)
        );
    }
    #[test]
    fn search_text_is_not_reinterpreted_as_motions_or_permissions() {
        let mut input = VimInput::default();
        assert_eq!(
            key(&mut input, "/"),
            Input::Command(KeyboardCommand::SearchStart(true))
        );
        for value in ["y", "n", "e", "t", "2"] {
            assert_eq!(
                key(&mut input, value),
                Input::Command(KeyboardCommand::SearchChar(value.chars().next().unwrap()))
            );
        }
        assert_eq!(input.hint().as_deref(), Some("/ynet2"));
        assert_eq!(
            key(&mut input, "enter"),
            Input::Command(KeyboardCommand::SearchAccept)
        );
        assert_eq!(
            key(&mut input, "n"),
            Input::Command(KeyboardCommand::SearchNext(false))
        );
    }
    #[test]
    fn shifted_motions_are_distinct_and_modified_shortcuts_pass_through() {
        let mut input = VimInput::default();
        for (key_name, command) in [
            ("W", KeyboardCommand::BigWordForward),
            ("E", KeyboardCommand::BigWordEnd),
            ("B", KeyboardCommand::BigWordBackward),
            ("H", KeyboardCommand::Viewport(0)),
            ("M", KeyboardCommand::Viewport(1)),
            ("L", KeyboardCommand::Viewport(2)),
            ("^", KeyboardCommand::FirstNonblank),
            ("$", KeyboardCommand::LineEnd),
            ("%", KeyboardCommand::MatchBracket),
            ("N", KeyboardCommand::SearchNext(true)),
        ] {
            assert_eq!(
                key(&mut input, key_name),
                Input::Command(command),
                "{key_name}"
            );
        }
        for chord in ["cmd-e", "alt-b", "ctrl-shift-f"] {
            assert_eq!(key(&mut input, chord), Input::Pass);
        }
        key(&mut input, "g");
        input.clear();
        assert_eq!(
            key(&mut input, "e"),
            Input::Command(KeyboardCommand::WordEnd)
        );
    }
}
