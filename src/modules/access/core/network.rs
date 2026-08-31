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
    "*.opencode.ai",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_covers_direct_provider_endpoints() {
        assert!(base_network_host_allowed("chatgpt.com"));
        assert!(base_network_host_allowed(
            "generativelanguage.googleapis.com"
        ));
        assert!(base_network_host_allowed("example-resource.ai.azure.com"));
        assert!(base_network_host_allowed("models.opencode.ai"));
        assert!(!base_network_host_allowed("chatgpt.com.example.org"));
    }
}
