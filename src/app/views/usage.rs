use super::super::FarcasterApp;
use crate::sessions::{SessionSummary, UsageSummary};

#[derive(Default)]
pub(super) struct ComposerUsage {
    pub context_used: Option<u64>,
    pub context_total: Option<u64>,
    pub context_percent: Option<f64>,
    pub aggregate: UsageSummary,
    pub cache_hit_rate: Option<f64>,
    pub weekly: Option<crate::agents::AccountUsageWindow>,
}

pub(super) fn composer_usage(app: &FarcasterApp) -> ComposerUsage {
    let root = app
        .sessions
        .all
        .root_for_path(app.snapshot.selected_session.as_deref());
    let descendants = root
        .map(|root| app.sessions.all.descendants(root))
        .unwrap_or_default();
    let selected = app.snapshot.selected_session.as_deref();
    let live = selected
        .filter(|path| Some(*path) == app.snapshot.live_session.as_deref())
        .and_then(|_| live_usage(&app.snapshot.stats));
    let usage_for = |session: &SessionSummary| {
        live.filter(|live| {
            selected == Some(session.path.as_path()) && live.total >= session.usage.total
        })
        .map(|live| UsageSummary {
            cost_micros: session.usage.cost_micros,
            ..live
        })
        .unwrap_or(session.usage)
    };
    let mut aggregate = root.map(&usage_for).unwrap_or_default();
    for (session, _) in &descendants {
        aggregate.add(usage_for(session));
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
        weekly: app
            .snapshot
            .account_usage
            .weekly
            .filter(|window| window.remaining_percent.is_finite()),
    }
}

fn live_usage(stats: &serde_json::Value) -> Option<UsageSummary> {
    let tokens = stats.get("tokens")?;
    let number = |key| tokens.get(key).and_then(serde_json::Value::as_u64);
    Some(UsageSummary {
        input: number("input")?,
        output: number("output")?,
        cache_read: number("cacheRead")?,
        cache_write: number("cacheWrite")?,
        total: number("totalTokens")?,
        cost_micros: 0,
    })
}

pub(super) fn has_meaningful_usage(usage: &ComposerUsage) -> bool {
    usage.context_used.is_some_and(|value| value > 0)
        || usage.aggregate.input > 0
        || usage.aggregate.output > 0
        || usage.aggregate.cost_micros > 0
        || usage.cache_hit_rate.is_some()
        || usage.weekly.is_some()
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
#[path = "usage_tests.rs"]
mod tests;
