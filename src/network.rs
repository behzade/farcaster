//! Harness-wide network destinations and optional proxy configuration.

use std::ffi::OsString;

const PROXY_ENVIRONMENT_NAMES: [&str; 4] =
    ["http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY"];

// Farcaster services exposed to the harness on the local machine.
const LOCAL_NETWORK_HOSTS: &[&str] = &["127.0.0.1", "::1", "localhost"];

// Built-in model APIs and subscription gateways used by coding harnesses.
const MODEL_SERVICE_HOSTS: &[&str] = &[
    "*.ai.azure.com",
    "*.amazonaws.com",
    "*.cognitiveservices.azure.com",
    "*.githubcopilot.com",
    "*.googleapis.com",
    "*.openai.azure.com",
    "ai-gateway.vercel.sh",
    "api.ant-ling.com",
    "api.anthropic.com",
    "api.cerebras.ai",
    "api.cloudflare.com",
    "api.cohere.ai",
    "api.cohere.com",
    "api.deepseek.com",
    "api.fireworks.ai",
    "api.groq.com",
    "api.individual.githubcopilot.com",
    "api.kimi.com",
    "api.minimax.io",
    "api.minimaxi.com",
    "api.mistral.ai",
    "api.moonshot.ai",
    "api.moonshot.cn",
    "api.openai.com",
    "api.openrouter.ai",
    "api.perplexity.ai",
    "api.together.ai",
    "api.together.xyz",
    "api.x.ai",
    "api.xiaomimimo.com",
    "api.z.ai",
    "chatgpt.com",
    "claude.ai",
    "gateway.ai.cloudflare.com",
    "inference.baseten.co",
    "integrate.api.nvidia.com",
    "open.bigmodel.cn",
    "opencode.ai",
    "openrouter.ai",
    "radius.pi.dev",
    "router.huggingface.co",
    "token-plan-ams.xiaomimimo.com",
    "token-plan-cn.xiaomimimo.com",
    "token-plan-sgp.xiaomimimo.com",
    "token-plan.ap-southeast-1.maas.aliyuncs.com",
    "token-plan.cn-beijing.maas.aliyuncs.com",
];

// Browser login, OAuth exchange, and token refresh endpoints.
const AUTHENTICATION_HOSTS: &[&str] = &[
    "accounts.google.com",
    "auth.kimi.com",
    "auth.openai.com",
    "auth.x.ai",
    "oauth2.googleapis.com",
    "platform.claude.com",
];

// Source hosts, language registries, package downloads, and build toolchains.
const DEVELOPMENT_HOSTS: &[&str] = &[
    "api.github.com",
    "cache.nixos.org",
    "codeload.github.com",
    "crates.io",
    "files.pythonhosted.org",
    "ghcr.io",
    "github.com",
    "index.crates.io",
    "nodejs.org",
    "objects.githubusercontent.com",
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

const NETWORK_HOST_GROUPS: &[&[&str]] = &[
    LOCAL_NETWORK_HOSTS,
    MODEL_SERVICE_HOSTS,
    AUTHENTICATION_HOSTS,
    DEVELOPMENT_HOSTS,
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

pub(crate) fn allowed_network_hosts() -> impl Iterator<Item = &'static str> {
    NETWORK_HOST_GROUPS
        .iter()
        .flat_map(|hosts| hosts.iter().copied())
}

pub(crate) const fn loopback_ports() -> &'static [u16] {
    LOOPBACK_PORTS
}

pub(crate) fn base_network_host_allowed(host: &str) -> bool {
    allowed_network_hosts().any(|allowed| {
        allowed == host
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
    fn baseline_covers_direct_provider_endpoints() {
        assert!(base_network_host_allowed("chatgpt.com"));
        assert!(base_network_host_allowed(
            "generativelanguage.googleapis.com"
        ));
        assert!(base_network_host_allowed("example-resource.ai.azure.com"));
        assert!(!base_network_host_allowed("chatgpt.com.example.org"));
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
