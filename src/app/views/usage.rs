//! Composer-footer usage projection and compact formatting.

use super::super::FarcasterApp;
use crate::sessions::{UsageSummary, descendant_sessions, root_session_for_path};

#[derive(Default)]
pub(super) struct ComposerUsage {
    pub context_used: Option<u64>,
    pub context_total: Option<u64>,
    pub context_percent: Option<f64>,
    pub aggregate: UsageSummary,
    pub cache_hit_rate: Option<f64>,
}

pub(super) fn composer_usage(app: &FarcasterApp) -> ComposerUsage {
    let root = root_session_for_path(&app.all_sessions, app.snapshot.selected_session.as_deref());
    let descendants = root
        .map(|root| descendant_sessions(&app.all_sessions, &root.id))
        .unwrap_or_default();
    let mut aggregate = root.map(|root| root.usage).unwrap_or_default();
    for (session, _) in &descendants {
        aggregate.add(session.usage);
    }

    let context = context_summary(visible_context_stats(
        &app.snapshot.stats,
        app.snapshot.conversation.running,
    ));
    let model_window = app
        .snapshot
        .session_identity()
        .model
        .map(|model| model.context_window)
        .filter(|window| *window > 0);
    let context_total = context.total.or(model_window);
    let context_percent = context.percent.or_else(|| {
        context
            .used
            .zip(context_total)
            .map(|(used, total)| used as f64 * 100.0 / total as f64)
    });

    ComposerUsage {
        context_used: context.used,
        context_total,
        context_percent,
        aggregate,
        cache_hit_rate: app
            .snapshot
            .conversation
            .average_cache_hit_rate
            .filter(|rate| rate.is_finite())
            .map(|rate| rate.clamp(0.0, 100.0)),
    }
}

pub(super) fn has_meaningful_usage(usage: &ComposerUsage) -> bool {
    usage.context_used.is_some_and(|value| value > 0)
        || usage.aggregate.input > 0
        || usage.aggregate.output > 0
        || usage.aggregate.cost_micros > 0
        || usage.cache_hit_rate.is_some()
}

pub(super) fn format_tokens(value: u64) -> String {
    if value < 1_000 {
        value.to_string()
    } else if value < 100_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else if value < 1_000_000 {
        format!("{}k", (value as f64 / 1_000.0).round() as u64)
    } else if value < 10_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else {
        format!("{}M", (value as f64 / 1_000_000.0).round() as u64)
    }
}

pub(super) fn format_cost(micros: u64) -> String {
    format!("${:.3}", micros as f64 / 1_000_000.0)
}

struct ContextSummary {
    percent: Option<f64>,
    used: Option<u64>,
    total: Option<u64>,
}

fn visible_context_stats(stats: &serde_json::Value, running: bool) -> Option<&serde_json::Value> {
    let context = stats.get("contextUsage");
    let meaningful = context
        .and_then(|context| context.get("tokens"))
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|tokens| tokens > 0)
        || context
            .and_then(|context| context.get("percent"))
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|percent| percent.is_finite() && percent > 0.0);
    (!running || meaningful).then_some(stats)
}

fn context_summary(stats: Option<&serde_json::Value>) -> ContextSummary {
    let context = stats.and_then(|stats| stats.get("contextUsage"));
    let used = context
        .and_then(|context| context.get("tokens"))
        .and_then(serde_json::Value::as_u64);
    let total = context
        .and_then(|context| context.get("contextWindow"))
        .and_then(serde_json::Value::as_u64)
        .filter(|total| *total > 0);
    let percent = context
        .and_then(|context| context.get("percent"))
        .and_then(serde_json::Value::as_f64)
        .filter(|percent| percent.is_finite())
        .or_else(|| match (used, total) {
            (Some(used), Some(total)) => Some(used as f64 * 100.0 / total as f64),
            _ => None,
        })
        .map(|percent| percent.clamp(0.0, 100.0));
    ContextSummary {
        percent,
        used,
        total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_context_hides_transient_zeroes() {
        let zero = serde_json::json!({
            "contextUsage": {"tokens": 0, "contextWindow": 200_000, "percent": 0.0}
        });
        let known = serde_json::json!({
            "contextUsage": {"tokens": 168_000, "contextWindow": 200_000, "percent": 84.0}
        });

        assert_eq!(visible_context_stats(&zero, true), None);
        assert_eq!(visible_context_stats(&zero, false), Some(&zero));
        assert_eq!(visible_context_stats(&known, true), Some(&known));
    }

    #[test]
    fn context_projection_handles_explicit_derived_and_partial_values() {
        let explicit = context_summary(Some(&serde_json::json!({
            "contextUsage": {"tokens": 160_000, "contextWindow": 200_000, "percent": 81.25}
        })));
        assert_eq!(explicit.percent, Some(81.25));
        assert_eq!(explicit.used, Some(160_000));
        assert_eq!(explicit.total, Some(200_000));

        let derived = context_summary(Some(&serde_json::json!({
            "contextUsage": {"tokens": 50, "contextWindow": 200}
        })));
        assert_eq!(derived.percent, Some(25.0));

        let partial = context_summary(Some(&serde_json::json!({
            "contextUsage": {"tokens": 25_000}
        })));
        assert_eq!(partial.percent, None);
        assert_eq!(partial.total, None);
    }

    #[test]
    fn footer_values_use_compact_stable_formatting() {
        assert_eq!(format_tokens(320), "320");
        assert_eq!(format_tokens(1_200), "1.2k");
        assert_eq!(format_tokens(14_600), "14.6k");
        assert_eq!(format_tokens(128_000), "128k");
        assert_eq!(format_tokens(2_000_000), "2.0M");
        assert_eq!(format_cost(18_000), "$0.018");
    }

    #[test]
    fn meaningful_usage_requires_a_nonzero_metric() {
        assert!(!has_meaningful_usage(&ComposerUsage::default()));

        let used = ComposerUsage {
            aggregate: UsageSummary {
                input: 1,
                ..UsageSummary::default()
            },
            ..ComposerUsage::default()
        };
        assert!(has_meaningful_usage(&used));
    }
}
