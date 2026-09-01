//! Harness-wide optional proxy configuration.

use std::ffi::OsString;

use super::super::NetworkConfiguration;

const PROXY_ENVIRONMENT_NAMES: [&str; 4] =
    ["http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY"];

pub(crate) fn configuration(
    environment: Option<&[(OsString, OsString)]>,
    app_proxy: Option<&str>,
) -> NetworkConfiguration {
    let inherited = environment.into_iter().flatten().any(|(name, value)| {
        PROXY_ENVIRONMENT_NAMES
            .iter()
            .any(|candidate| name == candidate)
            && !value.is_empty()
    });
    NetworkConfiguration {
        app_proxy: (!inherited)
            .then_some(app_proxy)
            .flatten()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
    }
}

pub(crate) fn validate_app_proxy(value: &str) -> Result<(), String> {
    let url = url::Url::parse(value.trim()).map_err(|_| "proxy URL is invalid".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("proxy URL scheme must be http or https".into());
    }
    if url.host().is_none() {
        return Err("proxy URL must include a host".into());
    }
    url.port_or_known_default()
        .ok_or_else(|| "proxy URL must include a valid port".to_owned())?;
    Ok(())
}

pub(crate) fn append_app_proxy_environment(
    environment: &mut Vec<(OsString, OsString)>,
    configuration: &NetworkConfiguration,
) {
    let Some(proxy) = configuration.app_proxy.as_ref() else {
        return;
    };
    environment.push((OsString::from("http_proxy"), OsString::from(proxy)));
    environment.push((OsString::from("https_proxy"), OsString::from(proxy)));
}

#[cfg(test)]
mod tests {
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
}
