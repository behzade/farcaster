use super::*;

fn environment(values: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    values
        .iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
}

#[test]
fn environment_proxy_takes_precedence_without_rewriting_environment() {
    let values = environment(&[("HTTPS_PROXY", "http://proxy.example:8080")]);
    assert_eq!(
        configuration(Some(&values), Some("http://app.example:3128")),
        NetworkConfiguration::default()
    );
}

#[test]
fn app_proxy_is_used_only_when_environment_has_none() {
    let configuration = configuration(None, Some("http://127.0.0.1:8080"));
    assert_eq!(
        configuration.app_proxy.as_deref(),
        Some("http://127.0.0.1:8080")
    );
    let mut environment = Vec::new();
    append_app_proxy_environment(&mut environment, &configuration);
    assert_eq!(
        environment,
        [
            (
                OsString::from("http_proxy"),
                OsString::from("http://127.0.0.1:8080")
            ),
            (
                OsString::from("https_proxy"),
                OsString::from("http://127.0.0.1:8080")
            ),
        ]
    );
}

#[test]
fn proxy_credentials_are_redacted() {
    let secret = "secret-value";
    let resolved = configuration(None, Some(&format!("http://user:{secret}@proxy.example")));
    assert!(!format!("{resolved:?}").contains(secret));
    let error = validate_app_proxy(&format!("ftp://user:{secret}@proxy.example"))
        .expect_err("unsupported proxy must fail");
    assert!(!error.contains(secret));
}

#[test]
fn malformed_proxy_is_rejected() {
    assert!(validate_app_proxy("not a URL").is_err());
    assert!(validate_app_proxy("socks5://proxy.example:1080").is_err());
}
