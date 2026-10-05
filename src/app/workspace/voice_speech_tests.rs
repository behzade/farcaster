use super::*;

#[test]
fn speaks_the_opening_and_keeps_details_in_chat() {
    assert_eq!(
        spoken_opening(
            "This keeps your editor tab around.\n\nThe map uses session IDs.\n```rust\nstate.tabs[id]\n```"
        ),
        Some("This keeps your editor tab around.".into())
    );
}

#[test]
fn skips_headings_and_code_when_selecting_speech() {
    assert_eq!(
        spoken_opening(
            "# Explanation\n\n```rust\nlet secret = 42;\n```\n\nIt stores **one** tab per thread."
        ),
        Some("It stores one tab per thread.".into())
    );
    assert_eq!(spoken_opening("```rust\nlet x = 1;\n```"), None);
}

#[test]
fn verbose_answers_stop_at_a_sentence_when_possible() {
    let answer = format!("It stores your tab. {}", "details ".repeat(100));
    assert_eq!(
        spoken_opening(&answer),
        Some("It stores your tab. There is more in the chat.".into())
    );
}
