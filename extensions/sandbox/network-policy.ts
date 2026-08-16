import type { NativeSandboxConfig } from "./sandbox-config.ts";

/** Returns the exact runtime proxy host set after applying machine network policy. */
export function runtimeNetworkHosts(
	config: NativeSandboxConfig,
	projectHosts: readonly string[],
): string[] {
	if (config.network?.enabled === false) return [];
	const denied = config.network?.deniedDomains ?? [];
	return [
		...new Set([
			...(config.network?.allowedDomains ?? []),
			...projectHosts,
		].filter((host) => !denied.some((rule) => networkRuleMatches(rule, host)))),
	].sort();
}

export function networkRuleMatches(rule: string, host: string): boolean {
	const normalizedRule = rule.toLowerCase();
	const normalizedHost = host.toLowerCase();
	if (normalizedRule === "*") return true;
	if (normalizedRule.startsWith("**.")) {
		const base = normalizedRule.slice(3);
		return normalizedHost === base || normalizedHost.endsWith(`.${base}`);
	}
	if (normalizedRule.startsWith("*.")) {
		const base = normalizedRule.slice(2);
		return normalizedHost !== base && normalizedHost.endsWith(`.${base}`);
	}
	return normalizedRule === normalizedHost;
}
