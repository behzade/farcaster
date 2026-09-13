use super::extensions::Model;

pub(crate) fn effort_rank(effort: &str) -> Option<u8> {
    match effort {
        "off" | "none" => Some(0),
        "minimal" => Some(1),
        "low" => Some(2),
        "medium" => Some(3),
        "high" => Some(4),
        "xhigh" => Some(5),
        "max" => Some(6),
        _ => None,
    }
}

pub(crate) fn model_efforts(model: &Model, fallback: &[String]) -> Vec<String> {
    if !model.reasoning {
        return Vec::new();
    }
    // Presentation ordering must not change the catalog's model-switch policy.
    let mut efforts = model.efforts.as_deref().unwrap_or(fallback).to_vec();
    efforts.sort_by(|left, right| {
        effort_rank(left)
            .unwrap_or(u8::MAX)
            .cmp(&effort_rank(right).unwrap_or(u8::MAX))
            .then_with(|| left.cmp(right))
    });
    efforts
}

#[cfg(test)]
#[path = "effort_tests.rs"]
mod tests;
