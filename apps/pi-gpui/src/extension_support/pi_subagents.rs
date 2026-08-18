//! Projection of pi-subagents conventions into GPUI-owned activity values.
//!
//! No runtime or view outside this adapter should know pi-subagents session
//! names or tool protocols.

use serde_json::Value;

pub(crate) fn role(title: &str) -> &str {
    title
        .strip_prefix("subagent-")
        .and_then(|rest| rest.split('-').next())
        .filter(|role| !role.is_empty())
        .unwrap_or(title)
}

pub(crate) fn tool_requires_input(name: &str, arguments: &Value) -> bool {
    if !name.eq_ignore_ascii_case("contact_supervisor") {
        return false;
    }
    matches!(
        arguments.get("reason").and_then(Value::as_str),
        Some("need_decision" | "interview_request" | "approval_required")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn projects_native_session_titles_to_generic_roles() {
        assert_eq!(role("subagent-reviewer-run-id"), "reviewer");
        assert_eq!(role("ordinary session"), "ordinary session");
    }

    #[test]
    fn projects_native_supervisor_requests_to_needs_input() {
        assert!(tool_requires_input(
            "contact_supervisor",
            &json!({"reason":"need_decision"})
        ));
        assert!(!tool_requires_input(
            "contact_supervisor",
            &json!({"reason":"status_update"})
        ));
        assert!(!tool_requires_input(
            "other_tool",
            &json!({"reason":"need_decision"})
        ));
    }
}
