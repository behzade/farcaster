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
#[path = "network_tests.rs"]
mod tests;
