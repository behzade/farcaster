//! Compact native summaries for file mutation tools.

use std::rc::Rc;

use crate::{
    assets::AppIcon,
    conversation::ToolPresentation,
    primitives::{ButtonTone, icon_button},
    theme::{MONO_FONT_FAMILY, THEME},
};
use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window,
    div, px,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EmbeddedDiffMode {
    Split,
    Unified,
}

pub(crate) type ExpandHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PreparedToolChange {
    additions: usize,
    deletions: usize,
}

pub(crate) fn render(
    presentation: &ToolPresentation,
    key: usize,
    _requested_mode: EmbeddedDiffMode,
    on_expand: Option<ExpandHandler>,
) -> AnyElement {
    let (path, prepared) = match presentation {
        ToolPresentation::Edit {
            path,
            diff,
            prepared,
            ..
        } => (
            path,
            prepared.get_or_init(|| {
                let _timing = crate::performance::OperationTiming::new(
                    crate::performance::OperationKind::ToolPreview,
                    diff.as_deref().map_or(0, str::len),
                );
                edit_metadata(diff.as_deref())
            }),
        ),
        ToolPresentation::Write {
            path,
            content,
            prepared,
        } => (
            path,
            prepared.get_or_init(|| {
                let _timing = crate::performance::OperationTiming::new(
                    crate::performance::OperationKind::ToolPreview,
                    content.len(),
                );
                write_metadata(content)
            }),
        ),
    };
    render_summary(path, *prepared, key, on_expand)
}

fn write_metadata(content: &str) -> PreparedToolChange {
    PreparedToolChange {
        additions: content.lines().count(),
        deletions: 0,
    }
}

fn edit_metadata(diff: Option<&str>) -> PreparedToolChange {
    let Some(diff) = diff else {
        return PreparedToolChange::default();
    };
    diff.lines()
        .fold(PreparedToolChange::default(), |mut metadata, line| {
            match line.as_bytes().first() {
                Some(b'+') => metadata.additions += 1,
                Some(b'-') => metadata.deletions += 1,
                _ => {}
            }
            metadata
        })
}

fn render_summary(
    path: &str,
    metadata: PreparedToolChange,
    key: usize,
    on_expand: Option<ExpandHandler>,
) -> AnyElement {
    let expand = on_expand.map(|handler| {
        icon_button(
            ("expand-tool-change", key),
            AppIcon::ArrowsOut,
            "Expand diff",
            ButtonTone::Quiet,
            move |window, cx| handler(window, cx),
        )
    });
    div()
        .id(("tool-change", key))
        .w_full()
        .min_w_0()
        .h(px(34.0))
        .px(THEME.space.sm)
        .flex()
        .items_center()
        .gap(THEME.space.sm)
        .border_y(THEME.border)
        .border_color(THEME.colors.border)
        .bg(THEME.colors.panel)
        .child(change_count(
            format!("+{}", metadata.additions),
            THEME.colors.success,
        ))
        .child(change_count(
            format!("-{}", metadata.deletions),
            THEME.colors.error,
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_family(MONO_FONT_FAMILY)
                .text_size(THEME.type_scale.caption)
                .text_color(THEME.colors.text)
                .child(path.to_owned()),
        )
        .children(expand)
        .into_any_element()
}

fn change_count(label: String, color: gpui::Rgba) -> impl IntoElement {
    div()
        .flex_none()
        .font_family(MONO_FONT_FAMILY)
        .text_size(THEME.type_scale.caption)
        .text_color(color)
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_summary_counts_added_and_deleted_lines() {
        assert_eq!(
            edit_metadata(Some(
                " 10 context\n- 11 old one\n- 12 old two\n+ 11 new one"
            )),
            PreparedToolChange {
                additions: 1,
                deletions: 2,
            }
        );
    }

    #[test]
    fn missing_edit_diff_has_zero_counts() {
        assert_eq!(edit_metadata(None), PreparedToolChange::default());
    }

    #[test]
    fn write_summary_counts_added_lines() {
        assert_eq!(
            write_metadata("first\nsecond\n"),
            PreparedToolChange {
                additions: 2,
                deletions: 0,
            }
        );
    }
}
