use std::{path::PathBuf, time::SystemTime};

use super::*;

#[test]
fn hover_details_include_model_effort_and_counts() {
    let mut session = SessionSummary::from_cached(
        "one".into(),
        PathBuf::from("/one.jsonl"),
        PathBuf::from("/project"),
        "Review the rail".into(),
        "Please inspect the sidebar hover card.".into(),
        String::new(),
        None,
        SystemTime::UNIX_EPOCH,
        4,
        UsageSummary {
            input: 1200,
            output: 80,
            cost_micros: 18_000,
            ..UsageSummary::default()
        },
        false,
        false,
        String::new(),
    );
    session.model = Some(("anthropic".into(), "claude-opus-4-5".into()));
    session.thinking_level = Some("high".into());

    let details = session_hover_details(&session, "Working", "2m", 1);
    assert_eq!(details.title, "Review the rail");
    assert!(
        details
            .rows
            .contains(&("Model".into(), "anthropic / claude-opus-4-5".into()))
    );
    assert!(details.rows.contains(&("Effort".into(), "High".into())));
    assert!(details.rows.contains(&("State".into(), "Working".into())));
    assert!(details.rows.contains(&("Messages".into(), "4".into())));
    assert!(
        details
            .rows
            .contains(&("Subagents".into(), "1 subagent".into()))
    );
    assert_eq!(
        details.preview.as_deref(),
        Some("Please inspect the sidebar hover card.")
    );
}

#[test]
fn panel_width_stops_at_the_window_midline() {
    assert_eq!(
        panel_width(px(1200.0)),
        px(600.0 - f32::from(THEME.layout.session_rail))
    );
    assert_eq!(panel_width(px(800.0)), px(MIN_PANEL_WIDTH));
    assert_eq!(panel_width(px(2000.0)), px(MAX_PANEL_WIDTH));
}
