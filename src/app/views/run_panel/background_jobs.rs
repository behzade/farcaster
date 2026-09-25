use gpui::{AnyElement, IntoElement, ParentElement as _, Styled as _, div};

use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{AppIconSize, app_icon},
    app::ui::theme::{MONO_FONT_FAMILY, theme},
    protocol::{BackgroundJob, BackgroundJobState},
};

pub(super) fn background_job_row(job: &BackgroundJob) -> AnyElement {
    div()
        .px(theme().size(2.0))
        .py(theme().size(3.0))
        .flex()
        .items_start()
        .gap(theme().size(7.0))
        .child(
            div()
                .size(theme().icons.inline)
                .flex_none()
                .text_color(background_job_color(job.state))
                .child(app_icon(
                    background_job_icon(job.state),
                    AppIconSize::Inline,
                )),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .gap(theme().size(2.0))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(theme().type_scale.caption)
                        .text_color(theme().colors.text)
                        .child(job.name.clone()),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family(MONO_FONT_FAMILY)
                        .text_size(theme().type_scale.caption)
                        .text_color(theme().colors.subtle)
                        .child(job.command.clone()),
                ),
        )
        .child(
            div()
                .flex_none()
                .text_size(theme().type_scale.caption)
                .text_color(background_job_color(job.state))
                .child(background_job_label(job)),
        )
        .into_any_element()
}

fn background_job_label(job: &BackgroundJob) -> String {
    match job.state {
        BackgroundJobState::Starting => "Starting".into(),
        BackgroundJobState::Running => "Running".into(),
        BackgroundJobState::Completed => "Complete".into(),
        BackgroundJobState::Exited => job
            .exit_code
            .map_or_else(|| "Exited".into(), |code| format!("Exit {code}")),
        BackgroundJobState::Failed => "Failed".into(),
    }
}

fn background_job_icon(state: BackgroundJobState) -> AppIcon {
    match state {
        BackgroundJobState::Starting | BackgroundJobState::Running => AppIcon::SpinnerGap,
        BackgroundJobState::Completed => AppIcon::CheckCircle,
        BackgroundJobState::Exited | BackgroundJobState::Failed => AppIcon::XCircle,
    }
}

fn background_job_color(state: BackgroundJobState) -> gpui::Rgba {
    match state {
        BackgroundJobState::Starting | BackgroundJobState::Running => theme().colors.accent,
        BackgroundJobState::Completed => theme().colors.success,
        BackgroundJobState::Exited | BackgroundJobState::Failed => theme().colors.error,
    }
}
