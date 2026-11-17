use crate::agents::accounts::{
    AccountAuthState, AccountConnection, AccountLimit, AccountQuotaValue, AccountsSnapshot,
    account_now,
};
use crate::app::{
    FarcasterApp, RuntimeCommand,
    ui::{
        assets::AppIcon,
        primitives::{
            AppIconSize, ButtonTone, Popover, app_icon, button, dropdown_content_button,
            floating_surface,
        },
        theme::theme,
    },
    workspace::SettingsTab,
};
use gpui::{
    Anchor, AnyElement, FontWeight, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, prelude::FluentBuilder as _,
};

fn visible_connections(snapshot: &AccountsSnapshot) -> Vec<&AccountConnection> {
    let mut rows: Vec<&AccountConnection> = Vec::new();
    for connection in snapshot.connections.iter().filter(|c| {
        c.available && c.source.incarnation.is_none() && c.auth != AccountAuthState::SignedOut
    }) {
        let identity = |c: &AccountConnection| {
            c.account_id
                .as_deref()
                .and_then(|id| snapshot.account(id))
                .map(|a| &a.identity)
        };
        // The same native credential source may be observed from several projects.
        // Collapse duplicate presentation only; do not merge account IDs or quota pools.
        if let Some(row) = rows.iter_mut().find(|c| {
            c.source.backend == connection.source.backend
                && c.source.profile_id == connection.source.profile_id
                && c.source.provider == connection.source.provider
                && c.source.credential_source == connection.source.credential_source
                && identity(c) == identity(connection)
        }) {
            if connection.observed_at > row.observed_at {
                *row = connection;
            }
        } else {
            rows.push(connection);
        }
    }
    rows.sort_by_key(|c| {
        (
            name(snapshot, c).to_lowercase(),
            c.source.backend.to_string(),
            c.source.profile_id.clone(),
        )
    });
    rows
}

fn name(snapshot: &AccountsSnapshot, connection: &AccountConnection) -> String {
    connection
        .account_id
        .as_deref()
        .and_then(|id| snapshot.account(id))
        .map(|account| account.identity.display_name().to_owned())
        .unwrap_or_else(|| connection.source.provider.clone())
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

pub(super) fn sidebar(app: &FarcasterApp, entity: WeakEntity<FarcasterApp>) -> AnyElement {
    let summary = visible_connections(&app.accounts)
        .into_iter()
        .filter(|c| !c.refresh_failed && c.auth != AccountAuthState::Unknown)
        .filter_map(|c| {
            weekly(c)
                .filter(|l| l.is_fresh(account_now()))
                .map(|l| (c, l))
        })
        .min_by(|(_, a), (_, b)| {
            a.remaining_percent
                .unwrap()
                .total_cmp(&b.remaining_percent.unwrap())
        })
        .map(|(c, limit)| (name(&app.accounts, c), percent(limit)));
    let content = div()
        .w_full()
        .flex()
        .flex_col()
        .items_start()
        .child(
            div()
                .flex()
                .items_center()
                .gap(theme().space.sm)
                .child(app_icon(AppIcon::UserFocus, AppIconSize::Control))
                .child("Accounts"),
        )
        .child(
            div()
                .h(theme().type_scale.line_body)
                .w_full()
                .min_w_0()
                .flex()
                .items_center()
                .gap(theme().space.sm)
                .text_size(theme().type_scale.caption)
                .text_color(theme().colors.muted)
                .when_some(summary, |row, (label, value)| {
                    row.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_ellipsis()
                            .overflow_hidden()
                            .child(label),
                    )
                    .child(div().flex_none().whitespace_nowrap().child(value))
                }),
        );
    let open = entity.clone();
    div()
        .px(theme().space.sm)
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
                    .h(theme().type_scale.line_body * 2.0 + theme().space.sm * 2.0)
                    .debug_selector(|| "accounts-trigger".into()),
                )
                .on_open_change(move |is_open, _, cx| {
                    if *is_open {
                        let _ = open
                            .update(cx, |app, cx| app.send(RuntimeCommand::RefreshAccounts, cx));
                    }
                })
                .content(move |state, window, cx| {
                    let Some(app) = entity.upgrade() else {
                        return div().into_any_element();
                    };
                    let accounts = app.read(cx).accounts.clone();
                    let profiles = app.read(cx).settings.harness_profiles.clone();
                    let manage = entity.clone();
                    let close = cx.entity().downgrade();
                    let _ = (state, window);
                    floating_surface()
                        .id("accounts-content")
                        .debug_selector(|| "accounts-content".into())
                        .w(theme().size(360.0))
                        .max_h(theme().size(420.0))
                        .overflow_y_scroll()
                        .p(theme().space.md)
                        .flex()
                        .flex_col()
                        .gap(theme().space.md)
                        .children(
                            visible_connections(&accounts)
                                .into_iter()
                                .map(|c| connection_row(&accounts, c, false, &profiles)),
                        )
                        .child(button(
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
                        ))
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
    let source = format!(
        "{}{}",
        crate::agents::backend_display_name(connection.source.backend),
        connection
            .source
            .profile_id
            .as_deref()
            .and_then(|id| profiles.get(id).ok())
            .map(|p| format!(" · {}", p.name))
            .unwrap_or_default()
    );
    let mut row = div()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(theme().space.xs)
        .child(
            div()
                .min_w_0()
                .text_size(theme().type_scale.body)
                .font_weight(FontWeight::MEDIUM)
                .text_ellipsis()
                .overflow_hidden()
                .child(name(snapshot, connection)),
        )
        .child(
            div()
                .text_size(theme().type_scale.caption)
                .text_color(theme().colors.muted)
                .child(source)
                .children(
                    identity
                        .and_then(|id| id.workspace.as_ref())
                        .map(|s| format!(" · {s}")),
                )
                .children(
                    identity
                        .and_then(|id| id.plan.as_ref())
                        .map(|s| format!(" · {s}")),
                ),
        );
    let limits = if detail {
        connection.limits.iter().collect()
    } else {
        weekly(connection).into_iter().collect::<Vec<_>>()
    };
    for limit in limits {
        let label = if limit.duration_seconds == Some(604800) {
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
        let stale = connection.refresh_failed
            || connection.auth == AccountAuthState::Unknown
            || !limit.is_fresh(account_now());
        row = row.child(
            div()
                .text_size(theme().type_scale.caption)
                .text_color(theme().colors.muted)
                .child(label)
                .when(stale, |row| {
                    row.child(format!(
                        " · {}",
                        observed_age(limit.observed_at, account_now())
                    ))
                })
                .when_some(limit.resets_at.and_then(reset_time), |row, time| {
                    row.child(format!(" · resets {time}"))
                }),
        );
    }
    if detail {
        let status = match connection.auth {
            AccountAuthState::Configured => Some("Configured"),
            AccountAuthState::Unknown => Some("Not verified"),
            AccountAuthState::SignedOut => Some("Signed out"),
            AccountAuthState::NeedsAuth => Some("Sign in required"),
            AccountAuthState::SignedIn => None,
        };
        row = row.children(status.map(|s| {
            div()
                .text_size(theme().type_scale.caption)
                .text_color(theme().colors.muted)
                .child(s)
        }));
    }
    row.into_any_element()
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
    let refresh = entity.clone();
    div()
        .flex()
        .flex_col()
        .gap(theme().space.md)
        .child(button(
            "accounts-refresh",
            "Refresh",
            ButtonTone::Quiet,
            true,
            move |_, cx| {
                let _ = refresh.update(cx, |app, cx| app.send(RuntimeCommand::RefreshAccounts, cx));
            },
        ))
        .children(
            visible_connections(&app.accounts)
                .into_iter()
                .map(|connection| {
                    let mut row = div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(theme().space.sm)
                        .child(connection_row(
                            &app.accounts,
                            connection,
                            true,
                            &app.settings.harness_profiles,
                        ));
                    if connection.capabilities.logout
                        && connection.auth == AccountAuthState::SignedIn
                        && let Some(identity) = connection
                            .account_id
                            .as_deref()
                            .and_then(|id| app.accounts.account(id))
                            .map(|a| a.identity.clone())
                    {
                        let source = connection.source.clone();
                        let logout = entity.clone();
                        row = row.child(button(
                            format!("account-logout-{}", connection.id),
                            "Sign out of shared CLI",
                            ButtonTone::Quiet,
                            app.settings.sign_in.active.is_none()
                                && !app.settings.sign_in.signing_out,
                            move |_, cx| {
                                let _ = logout.update(cx, |app, cx| {
                                    app.sign_out_account(source.clone(), identity.clone(), cx)
                                });
                            },
                        ));
                    }
                    row
                }),
        )
        .child(super::dialogs::settings::harness_sign_in::render(
            app, entity,
        ))
        .into_any_element()
}

#[cfg(test)]
#[path = "accounts_tests.rs"]
mod tests;
