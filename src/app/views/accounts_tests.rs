use super::*;
use crate::agents::{Backend, accounts::*};

fn accounts(count: usize) -> AccountsSnapshot {
    let mut accounts = AccountsSnapshot::default();
    for n in 0..count {
        accounts.apply(AccountObservation {
            source: AccountSource {
                backend: Backend::Codex,
                profile_id: None,
                context: format!("/project/{n}"),
                provider: "openai".into(),
                credential_source: format!("native:{n}"),
                incarnation: None,
            },
            identity: Some(AccountIdentity {
                authority: "openai".into(),
                email: Some(format!("account{n}@example.com")),
                ..Default::default()
            }),
            auth: AccountAuthState::SignedIn,
            limits: vec![AccountLimit {
                key: "weekly".into(),
                label: "Weekly".into(),
                pool_id: None,
                duration_seconds: Some(604800),
                remaining_percent: Some(20.),
                value: None,
                resets_at: None,
                observed_at: account_now(),
            }],
            complete_limits: true,
            limits_failed: false,
            observed_at: account_now(),
            capabilities: AccountCapabilities::default(),
        });
    }
    accounts
}

#[gpui::test]
fn accounts_sidebar_stays_fixed_and_popup_stays_bounded(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::accounts_sidebar_stays_fixed_and_popup_stays_bounded"
        ),
        cx,
        |cx, app, _, _| {
            cx.simulate_resize(gpui::size(gpui::px(1400.), gpui::px(900.)));
            let mut previous = None;
            for count in [0, 1, 20] {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.accounts = std::sync::Arc::new(accounts(count));
                        app.notify_session_rail_shell(cx);
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                cx.update(|window, cx| window.draw(cx).clear(cx));
                let bounds = cx
                    .debug_bounds("accounts-trigger")
                    .expect("accounts control");
                let footer = cx
                    .debug_bounds("session-sidebar-footer")
                    .expect("sidebar footer");
                assert_eq!(bounds.left(), footer.left());
                assert_eq!(bounds.right(), footer.right());
                assert_eq!(
                    *previous.get_or_insert(bounds),
                    bounds,
                    "account discovery cannot move the sidebar footer"
                );
                let heading = cx
                    .debug_bounds("accounts-heading")
                    .expect("accounts heading");
                let summary = cx
                    .debug_bounds("accounts-summary")
                    .expect("account summary");
                assert_eq!(heading.left(), crate::app::ui::grid::sidebar_inset());
                assert_eq!(summary.left(), heading.left());
                assert!(summary.top() >= heading.bottom());
                if count > 0 {
                    let provider = cx.debug_bounds("accounts-provider").expect("provider");
                    let identity = cx.debug_bounds("accounts-identity").expect("identity");
                    assert_eq!(provider.left(), identity.left());
                }
            }
            cx.simulate_click(previous.unwrap().center(), Default::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let popup = cx.debug_bounds("accounts-content").expect("accounts popup");
            assert!(popup.size.height <= theme().size(420.));
            assert_eq!(popup.size.width, theme().size(360.));
            let list = cx
                .debug_bounds("accounts-list")
                .expect("scrollable accounts");
            let manage = cx.debug_bounds("accounts-manage").expect("manage accounts");
            assert!(list.bottom() <= manage.top());
            assert!(manage.bottom() <= popup.bottom());
        },
    );
}

#[test]
fn accounts_same_email_in_different_stores_remains_distinct() {
    let mut snapshot = accounts(2);
    snapshot.accounts[1].identity.email = snapshot.accounts[0].identity.email.clone();
    assert_eq!(visible_connections(&snapshot).len(), 2);
}

#[test]
fn accounts_excludes_provider_rosters_but_keeps_known_cached_identities() {
    let mut snapshot = accounts(5);
    snapshot.connections[0].account_id = None;
    snapshot.connections[0].auth = AccountAuthState::Configured;
    snapshot.accounts[1].identity.email = None;
    snapshot.accounts[1].identity.label = Some("OpenAI API".into());
    snapshot.connections[1].auth = AccountAuthState::Configured;
    snapshot.connections[2].auth = AccountAuthState::Unknown;
    snapshot.connections[2].refresh_failed = true;
    snapshot.connections[3].auth = AccountAuthState::SignedOut;
    snapshot.accounts[4].identity.email = None;
    snapshot.accounts[4].identity.principal_id = Some("verified-principal".into());
    let rows = visible_connections(&snapshot);
    assert_eq!(rows.len(), 2);
    assert!(rows.contains(&&snapshot.connections[2]));
    assert!(rows.contains(&&snapshot.connections[4]));
}

#[test]
fn worker_only_identities_are_visible_and_the_same_store_account_appears_once() {
    let mut snapshot = accounts(1);
    snapshot.connections[0].source.incarnation = Some("worker-only".into());
    assert_eq!(visible_connections(&snapshot).len(), 1);
    let mut another = snapshot.connections[0].clone();
    another.id = "second-worker".into();
    another.source.incarnation = Some("second-worker".into());
    another.limits[0].observed_at += 10;
    another.limits[0].remaining_percent = Some(10.);
    snapshot.connections.push(another);
    let rows = visible_connections(&snapshot);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "second-worker");
    snapshot.connections[1].source.credential_source = "native:different-store".into();
    assert_eq!(visible_connections(&snapshot).len(), 2);
}

#[test]
fn sidebar_keeps_account_facts_during_rechecks_and_identity_before_first_quota() {
    let mut snapshot = accounts(1);
    let initial = sidebar_summary(&snapshot).unwrap();
    let expected = (initial.0.account_id.clone(), initial.1.cloned());
    snapshot.connections[0].auth = AccountAuthState::Unknown;
    snapshot.connections[0].refresh_failed = true;
    let pending = sidebar_summary(&snapshot).unwrap();
    assert_eq!((pending.0.account_id.clone(), pending.1.cloned()), expected);
    snapshot.connections[0].limits.clear();
    let identity_only = sidebar_summary(&snapshot).unwrap();
    assert_eq!(identity_only.0.account_id, expected.0);
    assert!(identity_only.1.is_none());
    snapshot.connections[0].auth = AccountAuthState::SignedOut;
    assert!(sidebar_summary(&snapshot).is_none());
}

#[test]
fn signed_in_connection_can_show_a_reported_plan_without_an_email() {
    let mut snapshot = accounts(1);
    snapshot.accounts[0].identity.email = None;
    snapshot.accounts[0].identity.plan = Some("Google AI Pro".into());
    assert_eq!(visible_connections(&snapshot).len(), 1);
    snapshot.connections[0].auth = AccountAuthState::Configured;
    assert!(visible_connections(&snapshot).is_empty());
}
