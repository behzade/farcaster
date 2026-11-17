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
                assert_eq!(
                    *previous.get_or_insert(bounds),
                    bounds,
                    "account discovery cannot move the sidebar footer"
                );
            }
            cx.simulate_click(previous.unwrap().center(), Default::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let popup = cx.debug_bounds("accounts-content").expect("accounts popup");
            assert!(popup.size.height <= theme().size(420.));
            assert_eq!(popup.size.width, theme().size(360.));
        },
    );
}

#[test]
fn accounts_same_email_in_different_stores_remains_distinct() {
    let mut snapshot = accounts(2);
    snapshot.accounts[1].identity.email = snapshot.accounts[0].identity.email.clone();
    assert_eq!(visible_connections(&snapshot).len(), 2);
}
