use crate::agents::accounts::{
    AccountAuthState, AccountConnection, AccountLimit, AccountQuotaValue, AccountsSnapshot,
    account_now,
};
use crate::app::{
    FarcasterApp, RuntimeCommand,
    ui::{
        assets::AppIcon,
        primitives::{AppIconSize, ButtonTone, Popover, app_icon, button, dropdown_content_button},
        theme::theme,
    },
    workspace::SettingsTab,
};
use gpui::{
    Anchor, AnyElement, FontWeight, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _,
};
use gpui_component::ThemeStyled as _;

fn provider_name(connection: &AccountConnection) -> &str {
    match connection.source.provider.as_str() {
        "openai" | "openai-codex" => "OpenAI",
        "anthropic" => "Anthropic",
        "google" | "google-gemini-cli" | "google-antigravity" => "Google",
        "cursor" => "Cursor",
        "opencode" => "OpenCode",
        "opencode-go" => "OpenCode Go",
        "xai" => "xAI",
        provider => provider,
    }
}

fn provider_icon(connection: &AccountConnection) -> AppIcon {
    match connection.source.provider.as_str() {
        "openai" | "openai-codex" => AppIcon::Codex,
        "anthropic" => AppIcon::Claude,
        "cursor" => AppIcon::Cursor,
        "google" | "google-gemini-cli" | "google-antigravity" => AppIcon::Google,
        "opencode" | "opencode-go" => AppIcon::OpenCode,
        _ => AppIcon::Key,
    }
}

fn plan_name(plan: &str) -> &str {
    match plan {
        "free" => "Free",
        "plus" => "Plus",
        "pro" => "Pro",
        "max" => "Max",
        "team" => "Team",
        "enterprise" => "Enterprise",
        _ => plan,
    }
}

fn visible_connections(snapshot: &AccountsSnapshot) -> Vec<&AccountConnection> {
    let mut rows: Vec<_> = snapshot
        .connections
        .iter()
        .filter(|c| c.available && c.auth != AccountAuthState::SignedOut)
        .filter(|c| {
            c.account_id
                .as_deref()
                .and_then(|id| snapshot.account(id))
                .is_some_and(|account| {
                    (c.auth == AccountAuthState::SignedIn
                        && account
                            .identity
                            .plan
                            .as_deref()
                            .is_some_and(|plan| !plan.trim().is_empty()))
                        || [
                            account.identity.email.as_deref(),
                            account.identity.principal_id.as_deref(),
                        ]
                        .into_iter()
                        .flatten()
                        .any(|value| !value.trim().is_empty())
                })
        })
        .collect();
    rows.sort_by_key(|c| {
        std::cmp::Reverse((
            c.auth != AccountAuthState::Unknown,
            c.limits.iter().map(|l| l.observed_at).max().unwrap_or(0),
            c.observed_at,
            c.source.incarnation.is_none(),
        ))
    });
    let mut distinct: Vec<&AccountConnection> = Vec::new();
    for connection in rows {
        if !distinct.iter().any(|other| {
            connection.source.same_store(&other.source)
                && connection.source.provider == other.source.provider
                && (connection.account_id == other.account_id
                    || connection
                        .account_id
                        .as_deref()
                        .and_then(|id| snapshot.account(id))
                        .zip(
                            other
                                .account_id
                                .as_deref()
                                .and_then(|id| snapshot.account(id)),
                        )
                        .is_some_and(|(a, b)| a.identity.same_store_identity(&b.identity)))
        }) {
            distinct.push(connection);
        }
    }
    distinct.sort_by_key(|c| {
        (
            name(snapshot, c).to_lowercase(),
            c.source.backend.to_string(),
            c.source.profile_id.clone(),
        )
    });
    distinct
}

fn name(snapshot: &AccountsSnapshot, connection: &AccountConnection) -> String {
    connection
        .account_id
        .as_deref()
        .and_then(|id| snapshot.account(id))
        .map(|account| account.identity.display_name().to_owned())
        .unwrap_or_else(|| provider_name(connection).to_owned())
}

fn weekly(connection: &AccountConnection) -> Option<&AccountLimit> {
    connection
        .limits
        .iter()
        .filter(|limit| limit.duration_seconds == Some(604800))
        .filter(|limit| limit.remaining_percent.is_some())
        .min_by(|a, b| {
            a.remaining_percent
                .unwrap()
                .total_cmp(&b.remaining_percent.unwrap())
        })
}

fn percent(limit: &AccountLimit) -> String {
    format!(
        "{:.0}% weekly left",
        limit.remaining_percent.unwrap_or_default()
    )
}

fn sidebar_summary(
    snapshot: &AccountsSnapshot,
) -> Option<(&AccountConnection, Option<&AccountLimit>)> {
    let connections = visible_connections(snapshot);
    connections
        .iter()
        .filter_map(|c| weekly(c).map(|l| (*c, l)))
        .min_by(|(_, a), (_, b)| {
            a.remaining_percent
                .unwrap()
                .total_cmp(&b.remaining_percent.unwrap())
        })
        .map(|(c, limit)| (c, Some(limit)))
        .or_else(|| connections.first().map(|c| (*c, None)))
}

pub(super) fn sidebar(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let summary = sidebar_summary(&app.accounts);
    let content = div()
        .w_full()
        .flex()
        .flex_col()
        .gap(theme().space.xs)
        .items_start()
        .child(
            div()
                .debug_selector(|| "accounts-heading".into())
                .h(theme().type_scale.line_body)
                .w_full()
                .text_size(theme().type_scale.caption)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme().colors.muted)
                .child("Accounts"),
        )
        .child(
            div()
                .debug_selector(|| "accounts-summary".into())
                .h(theme().type_scale.line_body * 2.0)
                .w_full()
                .min_w_0()
                .flex()
                .items_start()
                .gap(theme().space.xs)
                .when_some(summary, |row, (connection, value)| {
                    row.child(
                        crate::app::ui::grid::icon_slot()
                            .child(app_icon(provider_icon(connection), AppIconSize::Control)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(theme().space.sm)
                                    .child(
                                        div()
                                            .debug_selector(|| "accounts-provider".into())
                                            .flex_1()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(provider_name(connection).to_owned()),
                                    )
                                    .when_some(value, |row, limit| {
                                        row.child(
                                            div()
                                                .flex_none()
                                                .text_size(theme().type_scale.caption)
                                                .text_color(theme().colors.muted)
                                                .whitespace_nowrap()
                                                .child(percent(limit)),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "accounts-identity".into())
                                    .text_size(theme().type_scale.caption)
                                    .text_color(theme().colors.muted)
                                    .text_ellipsis()
                                    .overflow_hidden()
                                    .child(name(&app.accounts, connection)),
                            ),
                    )
                }),
        );
    div()
        .py(theme().space.xs)
        .child(
            Popover::new("accounts-popover")
                .anchor(Anchor::BottomLeft)
                .appearance(false)
                .trigger(
                    dropdown_content_button(
                        "accounts-trigger",
                        "Accounts",
                        content,
                        ButtonTone::Quiet,
                        true,
                    )
                    .w_full()
                    .px(crate::app::ui::grid::sidebar_inset())
                    .py(theme().space.xs)
                    .h(theme().type_scale.line_body * 3.0 + theme().space.xs * 3.0)
                    .debug_selector(|| "accounts-trigger".into()),
                )
                .content(move |state, window, cx| {
                    let Some(app) = entity.upgrade() else {
                        return div().into_any_element();
                    };
                    let accounts = app.read(cx).accounts.clone();
                    let profiles = app.read(cx).settings.harness_profiles.clone();
                    let manage = entity.clone();
                    let close = cx.entity().downgrade();
                    let _ = (state, window);
                    div()
                        .popover_style(cx)
                        .id("accounts-content")
                        .debug_selector(|| "accounts-content".into())
                        .w(theme().size(360.0))
                        .max_h(theme().size(420.0))
                        .p(theme().space.sm)
                        .flex()
                        .flex_col()
                        .gap(theme().space.sm)
                        .child(
                            div()
                                .id("accounts-list")
                                .debug_selector(|| "accounts-list".into())
                                .max_h(theme().size(360.0))
                                .overflow_y_scroll()
                                .flex()
                                .flex_col()
                                .children(visible_connections(&accounts).into_iter().map(|c| {
                                    div()
                                        .px(theme().space.sm)
                                        .py(theme().space.sm)
                                        .child(connection_row(&accounts, c, false, &profiles))
                                })),
                        )
                        .child(
                            button(
                                "accounts-manage",
                                "Manage accounts",
                                ButtonTone::Quiet,
                                true,
                                move |window, cx| {
                                    let _ = close.update(cx, |state, cx| state.dismiss(window, cx));
                                    let _ = manage.update(cx, |app, cx| {
                                        app.settings.tab = SettingsTab::Accounts;
                                        app.open_settings(window, cx);
                                    });
                                },
                            )
                            .debug_selector(|| "accounts-manage".into()),
                        )
                        .into_any_element()
                }),
        )
        .into_any_element()
}

fn connection_row(
    snapshot: &AccountsSnapshot,
    connection: &AccountConnection,
    detail: bool,
    profiles: &crate::agents::HarnessProfiles,
) -> AnyElement {
    let identity = connection
        .account_id
        .as_deref()
        .and_then(|id| snapshot.account(id))
        .map(|a| &a.identity);
    let metadata = connection
        .source
        .profile_id
        .as_deref()
        .and_then(|id| profiles.get(id).ok())
        .map(|p| p.name)
        .into_iter()
        .chain(identity.and_then(|id| id.workspace.clone()))
        .collect::<Vec<_>>()
        .join(" · ");
    let mut details: Vec<AnyElement> = Vec::new();
    if let Some(label) = identity
        .and_then(|id| id.email.as_ref().or(id.label.as_ref()))
        .filter(|label| {
            label.as_str() != connection.source.provider
                && label.as_str() != provider_name(connection)
        })
    {
        details.push(
            div()
                .min_w_0()
                .text_size(if detail {
                    theme().type_scale.body_small
                } else {
                    theme().type_scale.caption
                })
                .text_color(theme().colors.muted)
                .text_ellipsis()
                .overflow_hidden()
                .child(label.clone())
                .into_any_element(),
        );
    }
    if !metadata.is_empty() {
        details.push(
            div()
                .text_size(theme().type_scale.caption)
                .text_color(theme().colors.muted)
                .child(metadata)
                .into_any_element(),
        );
    }
    let limits = if detail {
        connection.limits.iter().collect()
    } else {
        weekly(connection).into_iter().collect::<Vec<_>>()
    };
    let stale = connection.refresh_failed || connection.auth == AccountAuthState::Unknown;
    for limit in limits {
        let mut label = if limit.duration_seconds == Some(604800) {
            if detail {
                format!("{} · {}", limit.label, percent(limit))
            } else {
                percent(limit)
            }
        } else if let Some(value) = &limit.value {
            match value {
                AccountQuotaValue::Balance { remaining, unit } => {
                    format!("{} · {remaining:.2} {unit}", limit.label)
                }
                AccountQuotaValue::Usage {
                    used,
                    limit: maximum,
                    unit,
                } => format!(
                    "{used:.2}{} {unit}",
                    maximum.map(|v| format!(" / {v:.2}")).unwrap_or_default()
                ),
                AccountQuotaValue::Unlimited => format!("{} · Unlimited", limit.label),
            }
        } else {
            format!(
                "{} · {:.0}% left",
                limit.label,
                limit.remaining_percent.unwrap_or_default()
            )
        };
        if stale {
            label.push_str(&format!(
                " · {}",
                observed_age(limit.observed_at, account_now())
            ));
        }
        let reset = limit.resets_at.and_then(reset_time);
        if detail {
            let bar = limit
                .value
                .is_none()
                .then_some(limit.remaining_percent)
                .flatten();
            details.push(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(theme().space.xs)
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(theme().space.sm)
                            .child(
                                div()
                                    .min_w_0()
                                    .text_size(theme().type_scale.caption)
                                    .text_color(theme().colors.muted)
                                    .child(label),
                            )
                            .when_some(reset, |row, time| {
                                row.child(
                                    div()
                                        .flex_none()
                                        .text_size(theme().type_scale.caption)
                                        .text_color(theme().colors.subtle)
                                        .whitespace_nowrap()
                                        .child(format!("Resets {time}")),
                                )
                            }),
                    )
                    .when_some(bar, |block, remaining| {
                        let ratio = (remaining / 100.0).clamp(0.0, 1.0) as f32;
                        block.child(
                            div()
                                .w_full()
                                .h(theme().size(4.0))
                                .rounded(theme().size(2.0))
                                .bg(theme().colors.highlight)
                                .overflow_hidden()
                                .child(
                                    div()
                                        .h_full()
                                        .w(gpui::relative(ratio))
                                        .rounded(theme().size(2.0))
                                        .bg(if remaining <= 15.0 {
                                            theme().colors.warning
                                        } else {
                                            theme().colors.indicator
                                        }),
                                ),
                        )
                    })
                    .into_any_element(),
            );
        } else {
            if let Some(time) = reset {
                label.push_str(&format!(" · resets {time}"));
            }
            details.push(
                div()
                    .text_size(theme().type_scale.caption)
                    .text_color(theme().colors.muted)
                    .child(label)
                    .into_any_element(),
            );
        }
    }
    if detail {
        let status = match connection.auth {
            AccountAuthState::Configured => Some("Configured"),
            AccountAuthState::Unknown => Some("Not verified"),
            AccountAuthState::SignedOut => Some("Signed out"),
            AccountAuthState::NeedsAuth => Some("Sign in required"),
            AccountAuthState::SignedIn => None,
        };
        if let Some(status) = status {
            details.push(
                div()
                    .text_size(theme().type_scale.caption)
                    .text_color(theme().colors.muted)
                    .child(status)
                    .into_any_element(),
            );
        }
    }
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(theme().space.xs)
        .child(
            div()
                .w_full()
                .min_w_0()
                .flex()
                .items_center()
                .gap(theme().space.sm)
                .text_size(theme().type_scale.body)
                .child(
                    crate::app::ui::grid::icon_slot()
                        .child(app_icon(provider_icon(connection), AppIconSize::Control)),
                )
                .child(
                    div()
                        .font_weight(if detail {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::MEDIUM
                        })
                        .child(provider_name(connection).to_owned()),
                )
                .when_some(identity.and_then(|id| id.plan.as_deref()), |row, plan| {
                    row.child(
                        div()
                            .px(theme().size(6.0))
                            .py(theme().size(1.0))
                            .rounded(theme().size(4.0))
                            .bg(theme().colors.highlight)
                            .text_size(theme().type_scale.caption)
                            .text_color(theme().colors.muted)
                            .child(plan_name(plan).to_owned()),
                    )
                })
                .child(div().flex_1())
                .child(
                    div()
                        .flex_none()
                        .text_size(theme().type_scale.caption)
                        .text_color(theme().colors.muted)
                        .child(crate::agents::backend_display_name(
                            connection.source.backend,
                        )),
                ),
        )
        .when(!details.is_empty(), |row| {
            row.child(
                div()
                    .w_full()
                    .min_w_0()
                    .pl(theme().icons.control + theme().space.sm)
                    .flex()
                    .flex_col()
                    .gap(theme().space.xs)
                    .children(details),
            )
        })
        .into_any_element()
}

fn observed_age(at: i64, now: i64) -> String {
    let age = now.saturating_sub(at).max(0);
    if age < 60 {
        "just observed".into()
    } else if age < 3600 {
        format!("{}m ago", age / 60)
    } else if age < 86400 {
        format!("{}h ago", age / 3600)
    } else {
        format!("{}d ago", age / 86400)
    }
}

fn reset_time(at: i64) -> Option<String> {
    let value = time::OffsetDateTime::from_unix_timestamp(at).ok()?;
    let format = time::format_description::parse_borrowed::<3>(
        "[month repr:short] [day padding:none] [hour]:[minute] UTC",
    )
    .ok()?;
    value.format(&format).ok()
}

pub(super) fn settings(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    if app.settings.sign_in.terminal.is_some() {
        return super::dialogs::settings::harness_sign_in::render_terminal(app, entity);
    }
    let refresh = entity.clone();
    let connections = visible_connections(&app.accounts);
    div()
        .debug_selector(|| "settings-accounts".into())
        .w_full()
        .flex()
        .flex_col()
        .gap(theme().size(24.0))
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(theme().space.sm)
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(theme().space.sm)
                        .child(
                            div()
                                .debug_selector(|| "accounts-section-heading".into())
                                .text_size(theme().type_scale.reading)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Connected accounts"),
                        )
                        .child(
                            button(
                                "accounts-refresh",
                                "Refresh",
                                ButtonTone::Neutral,
                                true,
                                move |_, cx| {
                                    let _ = refresh.update(cx, |app, cx| {
                                        app.send(RuntimeCommand::RefreshAccounts, cx)
                                    });
                                },
                            )
                            .icon(AppIcon::ArrowsClockwise)
                            .flex_none()
                            .debug_selector(|| "accounts-refresh".into()),
                        ),
                )
                .when(connections.is_empty(), |section| {
                    section.child(
                        div()
                            .text_size(theme().type_scale.body_small)
                            .text_color(theme().colors.muted)
                            .child("No connected accounts yet. Sign in below or refresh to check local CLI credentials."),
                    )
                })
                .children(connections.into_iter().enumerate().map(|(index, connection)| {
                    let mut actions: Vec<AnyElement> = Vec::new();
                    if let Some(url) = connection
                        .account_id
                        .as_deref()
                        .and_then(|id| app.accounts.account(id))
                        .and_then(|a| a.identity.upgrade_url.clone())
                    {
                        actions.push(
                            button(
                                format!("account-plan-{}", connection.id),
                                "Upgrade plan",
                                ButtonTone::Neutral,
                                true,
                                move |_, cx| cx.open_url(&url),
                            )
                            .shadow(crate::app::ui::primitives::control_shadow())
                            .icon(AppIcon::ArrowSquareOut)
                            .into_any_element(),
                        );
                    }
                    if connection.capabilities.logout
                        && connection.source.incarnation.is_none()
                        && connection.auth == AccountAuthState::SignedIn
                        && let Some(identity) = connection
                            .account_id
                            .as_deref()
                            .and_then(|id| app.accounts.account(id))
                            .map(|a| a.identity.clone())
                    {
                        let source = connection.source.clone();
                        let logout = entity.clone();
                        actions.push(
                            button(
                                format!("account-logout-{}", connection.id),
                                "Sign out of shared CLI",
                                ButtonTone::Neutral,
                                app.settings.sign_in.active.is_none()
                                    && app.settings.sign_in.terminal.is_none()
                                    && !app.settings.sign_in.signing_out,
                                move |_, cx| {
                                    let _ = logout.update(cx, |app, cx| {
                                        app.sign_out_account(source.clone(), identity.clone(), cx)
                                    });
                                },
                            )
                            .shadow(crate::app::ui::primitives::control_shadow())
                            .icon(AppIcon::SignOut)
                            .into_any_element(),
                        );
                    }
                    div()
                        .debug_selector(move || format!("account-card-{index}"))
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(theme().space.sm)
                        .p(theme().size(12.0))
                        .rounded(theme().size(6.0))
                        .bg(theme().colors.surface)
                        .child(connection_row(
                            &app.accounts,
                            connection,
                            true,
                            &app.settings.harness_profiles,
                        ))
                        .when(!actions.is_empty(), |card| {
                            card.child(
                                div()
                                    .w_full()
                                    .pl(theme().icons.control + theme().space.sm)
                                    .pt(theme().space.xs)
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap(theme().space.sm)
                                    .children(actions),
                            )
                        })
                })),
        )
        .child(super::dialogs::settings::harness_sign_in::render(
            app, entity,
        ))
        .into_any_element()
}

#[cfg(test)]
#[path = "accounts_tests.rs"]
mod tests;
