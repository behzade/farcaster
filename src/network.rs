//! Harness-wide network destinations and optional proxy configuration.

use std::ffi::OsString;

const PROXY_ENVIRONMENT_NAMES: [&str; 4] =
    ["http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY"];

const ALLOWED_NETWORK_HOSTS: &[&str] = &[
    "127.0.0.1",
    "::1",
    "localhost",
    "*.amazonaws.com",
    "*.openai.azure.com",
    "api.anthropic.com",
    "api.cohere.ai",
    "api.cohere.com",
    "api.deepseek.com",
    "api.fireworks.ai",
    "api.github.com",
    "api.groq.com",
    "api.mistral.ai",
    "api.openai.com",
    "api.openrouter.ai",
    "api.perplexity.ai",
    "api.together.xyz",
    "api.x.ai",
    "cache.nixos.org",
    "codeload.github.com",
    "crates.io",
    "files.pythonhosted.org",
    "generativelanguage.googleapis.com",
    "aiplatform.googleapis.com",
    "ghcr.io",
    "github.com",
    "index.crates.io",
    "nodejs.org",
    "oauth2.googleapis.com",
    "objects.githubusercontent.com",
    "openrouter.ai",
    "pkg-containers.githubusercontent.com",
    "proxy.golang.org",
    "pypi.org",
    "registry.npmjs.org",
    "release-assets.githubusercontent.com",
    "repo.maven.apache.org",
    "static.crates.io",
    "static.rust-lang.org",
    "storage.googleapis.com",
];
const LOOPBACK_PORTS: &[u16] = &[8765];

#[derive(Clone, Default, Eq, PartialEq)]
pub(crate) struct NetworkConfiguration {
    pub(crate) proxy_hosts: Vec<String>,
    pub(crate) proxy_loopback_ports: Vec<u16>,
    pub(crate) app_proxy: Option<String>,
}

impl std::fmt::Debug for NetworkConfiguration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NetworkConfiguration")
            .field("proxy_hosts", &self.proxy_hosts)
            .field("proxy_loopback_ports", &self.proxy_loopback_ports)
            .field("app_proxy", &self.app_proxy.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

pub(crate) fn configuration(
    environment: Option<&[(OsString, OsString)]>,
    app_proxy: Option<&str>,
    sandboxed: bool,
) -> Result<NetworkConfiguration, String> {
    let environment_proxies = environment
        .into_iter()
        .flatten()
        .filter(|(name, value)| {
            PROXY_ENVIRONMENT_NAMES
                .iter()
                .any(|candidate| name == candidate)
                && !value.is_empty()
        })
        .map(|(_, value)| {
            value
                .to_str()
                .ok_or_else(|| "proxy environment value is not valid UTF-8".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let app_proxy = if environment_proxies.is_empty() {
        app_proxy.map(str::trim).filter(|value| !value.is_empty())
    } else {
        None
    };
    let proxies = if environment_proxies.is_empty() {
        app_proxy.into_iter().collect::<Vec<_>>()
    } else {
        environment_proxies
    };

    let mut configuration = NetworkConfiguration {
        app_proxy: app_proxy.map(str::to_owned),
        ..Default::default()
    };
    if !sandboxed {
        return Ok(configuration);
    }
    for value in proxies {
        let destination = proxy_destination(value)?;
        match destination {
            ProxyDestination::Host(host) => configuration.proxy_hosts.push(host),
            ProxyDestination::Loopback(port) => {
                configuration.proxy_loopback_ports.push(port);
            }
        }
    }
    configuration.proxy_hosts.sort();
    configuration.proxy_hosts.dedup();
    configuration.proxy_loopback_ports.sort_unstable();
    configuration.proxy_loopback_ports.dedup();
    Ok(configuration)
}

pub(crate) fn validate_app_proxy(value: &str) -> Result<(), String> {
    proxy_destination(value.trim()).map(|_| ())
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

pub(crate) const fn allowed_network_hosts() -> &'static [&'static str] {
    ALLOWED_NETWORK_HOSTS
}

pub(crate) const fn loopback_ports() -> &'static [u16] {
    LOOPBACK_PORTS
}

pub(crate) fn base_network_host_allowed(host: &str) -> bool {
    ALLOWED_NETWORK_HOSTS.iter().any(|allowed| {
        *allowed == host
            || allowed.strip_prefix("*.").is_some_and(|suffix| {
                host.strip_suffix(suffix)
                    .is_some_and(|prefix| prefix.ends_with('.'))
            })
    })
}

pub(crate) fn base_loopback_port_allowed(port: u16) -> bool {
    LOOPBACK_PORTS.contains(&port)
}

enum ProxyDestination {
    Host(String),
    Loopback(u16),
}

fn proxy_destination(value: &str) -> Result<ProxyDestination, String> {
    let url = url::Url::parse(value).map_err(|_| "proxy URL is invalid".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("proxy URL scheme must be http or https".into());
    }
    let host = url
        .host()
        .ok_or_else(|| "proxy URL must include a host".to_owned())?;
    let (host, loopback) = match host {
        url::Host::Domain(host) => (host.to_ascii_lowercase(), host == "localhost"),
        url::Host::Ipv4(address) => (address.to_string(), address.is_loopback()),
        url::Host::Ipv6(address) => (address.to_string(), address.is_loopback()),
    };
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "proxy URL must include a valid port".to_owned())?;
    if loopback {
        Ok(ProxyDestination::Loopback(port))
    } else {
        Ok(ProxyDestination::Host(host))
    }
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
    fn environment_proxy_takes_precedence_without_rewriting_environment() -> Result<(), String> {
        let values = environment(&[("HTTPS_PROXY", "http://proxy.example:8080")]);
        let configuration = configuration(Some(&values), Some("http://app.example:3128"), true)?;
        assert_eq!(configuration.proxy_hosts, ["proxy.example"]);
        assert_eq!(configuration.app_proxy, None);
        Ok(())
    }

    #[test]
    fn app_proxy_is_used_only_when_environment_has_none() -> Result<(), String> {
        let configuration = configuration(None, Some("http://127.0.0.1:8080"), true)?;
        assert_eq!(configuration.proxy_loopback_ports, [8080]);
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
        Ok(())
    }

    #[test]
    fn ipv6_loopback_proxy_uses_its_exact_port() -> Result<(), String> {
        let configuration = configuration(None, Some("http://[::1]:3128"), true)?;
        assert_eq!(configuration.proxy_loopback_ports, [3128]);
        Ok(())
    }

    #[test]
    fn proxy_credentials_are_redacted() -> Result<(), String> {
        let secret = "secret-value";
        let resolved = configuration(
            None,
            Some(&format!("http://user:{secret}@proxy.example")),
            true,
        )?;
        assert!(!format!("{resolved:?}").contains(secret));
        let error = configuration(
            None,
            Some(&format!("ftp://user:{secret}@proxy.example")),
            true,
        )
        .expect_err("unsupported proxy must fail");
        assert!(!error.contains(secret));
        Ok(())
    }

    #[test]
    fn full_network_does_not_validate_inherited_proxy_syntax() -> Result<(), String> {
        let values = environment(&[("https_proxy", "harness-specific-proxy")]);
        assert_eq!(
            configuration(Some(&values), Some("http://app.example"), false)?,
            NetworkConfiguration::default()
        );
        Ok(())
    }

    #[test]
    fn malformed_proxy_is_rejected() {
        assert!(validate_app_proxy("not a URL").is_err());
        assert!(validate_app_proxy("socks5://proxy.example:1080").is_err());
    }
}
