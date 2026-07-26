import { homedir } from "node:os";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import ignore from "ignore";
import type {
	DebugLogger,
	PermissionDecision,
	PermissionRule,
	PermissionRules,
	PermissionSettingsFile,
	PermissionSource,
	PolicyOverrideRule,
	PolicyRule,
	ToolCallSummary,
} from "../types.js";
import { deepEqual } from "../utils/json.js";
import { toPosixPath } from "../utils/path.js";

export const PATH_TOOLS = new Set(["read", "edit", "write"]);

export function buildPermissionRules(
	settings: PermissionSettingsFile | undefined,
	source: PermissionSource,
	settingsPath: string,
	logDebug: DebugLogger,
): PermissionRules {
	const permissions = settings?.permissions;
	const allow = extractPermissionList(permissions?.allow);
	const ask = extractPermissionList(permissions?.ask);
	const deny = extractPermissionList(permissions?.deny);

	return {
		allow: allow
			.map((rule) => compilePermissionRule(rule, "allow", source, settingsPath, logDebug))
			.filter((rule): rule is PermissionRule => Boolean(rule)),
		ask: ask
			.map((rule) => compilePermissionRule(rule, "ask", source, settingsPath, logDebug))
			.filter((rule): rule is PermissionRule => Boolean(rule)),
		deny: deny
			.map((rule) => compilePermissionRule(rule, "deny", source, settingsPath, logDebug))
			.filter((rule): rule is PermissionRule => Boolean(rule)),
	};
}

export function buildPolicyRules(
	settings: PermissionSettingsFile | undefined,
	source: PermissionSource,
	settingsPath: string,
	logDebug: DebugLogger,
): PolicyRule[] {
	const preflight = settings?.preflight;
	if (!preflight || typeof preflight !== "object") return [];

	const entries = extractPolicyEntries((preflight as Record<string, unknown>).llmRules, logDebug);
	if (entries.length === 0) return [];

	return entries.map((entry) => ({
		tool: entry.tool,
		policy: entry.policy,
		source,
		settingsPath,
		settingsDir: dirname(settingsPath),
	}));
}

export function buildPolicyOverrides(
	settings: PermissionSettingsFile | undefined,
	source: PermissionSource,
	settingsPath: string,
	logDebug: DebugLogger,
): PolicyOverrideRule[] {
	const preflight = settings?.preflight;
	if (!preflight || typeof preflight !== "object") return [];
	const overrides = extractPermissionList((preflight as Record<string, unknown>).policyOverrides);

	return overrides
		.map((rule) => compilePolicyOverrideRule(rule, source, settingsPath, logDebug))
		.filter((rule): rule is PolicyOverrideRule => Boolean(rule));
}

export function extractPermissionList(value: unknown): string[] {
	if (!Array.isArray(value)) return [];
	return value
		.filter((item): item is string => typeof item === "string")
		.map((item) => item.trim())
		.filter((item) => item.length > 0);
}

export function getPolicyRulesForTool(toolName: string, rules: PolicyRule[]): string[] {
	const normalizedTool = normalizeToolName(toolName);
	const seen = new Set<string>();
	const policies: string[] = [];

	for (const rule of rules) {
		if (rule.tool !== "*" && rule.tool !== normalizedTool) continue;
		if (seen.has(rule.policy)) continue;
		seen.add(rule.policy);
		policies.push(rule.policy);
	}

	return policies;
}

export function getPolicyRuleBucketsForTool(
	toolName: string,
	rules: PolicyRule[],
): { global: string[]; tool: string[] } {
	const normalizedTool = normalizeToolName(toolName);
	const global: string[] = [];
	const tool: string[] = [];
	const globalSeen = new Set<string>();
	const toolSeen = new Set<string>();

	for (const rule of rules) {
		if (rule.tool === "*") {
			if (globalSeen.has(rule.policy)) continue;
			globalSeen.add(rule.policy);
			global.push(rule.policy);
			continue;
		}
		if (rule.tool !== normalizedTool) continue;
		if (toolSeen.has(rule.policy)) continue;
		toolSeen.add(rule.policy);
		tool.push(rule.policy);
	}

	return { global, tool };
}

export function getPermissionRulePatternsForTool(
	toolName: string,
	rules: PermissionRule[],
): string[] {
	const normalizedTool = normalizeToolName(toolName);
	const seen = new Set<string>();
	const patterns: string[] = [];

	for (const rule of rules) {
		if (!matchesRuleTool(rule.tool, normalizedTool)) continue;
		if (seen.has(rule.raw)) continue;
		seen.add(rule.raw);
		patterns.push(rule.raw);
	}

	return patterns;
}

export function getPolicyOverridesForTool(toolName: string, rules: PolicyOverrideRule[]): string[] {
	const normalizedTool = normalizeToolName(toolName);
	const seen = new Set<string>();
	const overrides: string[] = [];

	for (const rule of rules) {
		if (!matchesRuleTool(rule.tool, normalizedTool)) continue;
		if (seen.has(rule.raw)) continue;
		seen.add(rule.raw);
		overrides.push(rule.raw);
	}

	return overrides;
}

function matchesRuleTool(ruleTool: string, normalizedTool: string): boolean {
	return ruleTool === "*" || ruleTool === normalizedTool;
}

function extractPolicyEntries(
	value: unknown,
	logDebug: DebugLogger,
): Array<{ tool: string; policy: string }> {
	if (!value) return [];
	if (Array.isArray(value)) {
		return extractLegacyPolicyEntries(value, logDebug);
	}
	if (typeof value !== "object") {
		logDebug("Ignored invalid llmRules: expected object or array.");
		return [];
	}

	return extractToolScopedPolicyEntries(value as Record<string, unknown>, logDebug);
}

function extractToolScopedPolicyEntries(
	value: Record<string, unknown>,
	logDebug: DebugLogger,
): Array<{ tool: string; policy: string }> {
	const entries: Array<{ tool: string; policy: string }> = [];

	for (const [tool, rules] of Object.entries(value)) {
		const normalizedTool = normalizeToolName(tool);
		if (!normalizedTool) {
			logDebug("Ignored llmRules entry with empty tool name.");
			continue;
		}
		const policies = extractPolicyValues(rules, logDebug);
		for (const policy of policies) {
			entries.push({ tool: normalizedTool, policy });
		}
	}

	return entries;
}

function extractLegacyPolicyEntries(
	value: unknown[],
	logDebug: DebugLogger,
): Array<{ tool: string; policy: string }> {
	const entries: Array<{ tool: string; policy: string }> = [];

	for (const item of value) {
		if (typeof item === "string") {
			const policy = item.trim();
			if (policy) {
				entries.push({ tool: "*", policy });
			}
			continue;
		}

		if (!item || typeof item !== "object") {
			logDebug("Ignored invalid legacy llmRules entry.");
			continue;
		}

		const record = item as {
			pattern?: unknown;
			policy?: unknown;
			tool?: unknown;
		};
		const policy = typeof record.policy === "string" ? record.policy.trim() : "";
		if (!policy) {
			logDebug("Ignored legacy llmRules entry without policy text.");
			continue;
		}

		if (typeof record.tool === "string" && record.tool.trim()) {
			entries.push({ tool: normalizeToolName(record.tool), policy });
			continue;
		}

		if (typeof record.pattern === "string" && record.pattern.trim()) {
			const parsed = parseToolPattern(record.pattern);
			if (!parsed) {
				logDebug(`Ignored legacy llmRules entry with invalid pattern: ${record.pattern}`);
				continue;
			}
			entries.push({ tool: normalizeToolName(parsed.tool), policy });
			continue;
		}

		entries.push({ tool: "*", policy });
	}

	return entries;
}

function extractPolicyValues(value: unknown, logDebug: DebugLogger): string[] {
	if (typeof value === "string") {
		const trimmed = value.trim();
		return trimmed ? [trimmed] : [];
	}
	if (!Array.isArray(value)) {
		if (value !== undefined) {
			logDebug("Ignored invalid llmRules tool entry: expected array or string.");
		}
		return [];
	}

	const policies: string[] = [];
	for (const item of value) {
		if (typeof item === "string") {
			const trimmed = item.trim();
			if (trimmed) policies.push(trimmed);
			continue;
		}
		if (item && typeof item === "object") {
			const record = item as { policy?: unknown };
			if (typeof record.policy === "string" && record.policy.trim()) {
				policies.push(record.policy.trim());
				continue;
			}
		}
		logDebug("Ignored invalid llmRules policy entry.");
	}
	return policies;
}

export function compilePermissionRule(
	raw: string,
	kind: PermissionDecision,
	source: PermissionSource,
	settingsPath: string,
	logDebug: DebugLogger,
): PermissionRule | undefined {
	const parsed = parseToolPattern(raw);
	if (!parsed) {
		logDebug(`Ignored invalid rule: ${raw}`);
		return undefined;
	}
	const tool = normalizeToolName(parsed.tool);
	const specifier = normalizeSpecifier(parsed.specifier);
	const argsMatch = parseArgsMatch(tool, specifier, logDebug);
	if (specifier && !isKnownTool(tool) && argsMatch === undefined) {
		logDebug(`Ignored rule with unsupported args: ${raw}`);
		return undefined;
	}
	return {
		kind,
		raw,
		tool,
		specifier,
		source,
		settingsPath,
		settingsDir: dirname(settingsPath),
		argsMatch,
	};
}

export function compilePolicyOverrideRule(
	pattern: string,
	source: PermissionSource,
	settingsPath: string,
	logDebug: DebugLogger,
): PolicyOverrideRule | undefined {
	const parsed = parseToolPattern(pattern);
	if (!parsed) {
		logDebug(`Ignored invalid policy override: ${pattern}`);
		return undefined;
	}
	const tool = normalizeToolName(parsed.tool);
	const specifier = normalizeSpecifier(parsed.specifier);
	const argsMatch = parseArgsMatch(tool, specifier, logDebug);
	if (specifier && !isKnownTool(tool) && argsMatch === undefined) {
		logDebug(`Ignored policy override with unsupported args: ${pattern}`);
		return undefined;
	}
	return {
		raw: pattern,
		tool,
		specifier,
		source,
		settingsPath,
		settingsDir: dirname(settingsPath),
		argsMatch,
	};
}

export function parseToolPattern(value: string): { tool: string; specifier?: string } | undefined {
	const trimmed = value.trim();
	if (!trimmed) return undefined;
	const openIndex = trimmed.indexOf("(");
	if (openIndex === -1 || !trimmed.endsWith(")")) {
		return { tool: trimmed };
	}
	const tool = trimmed.slice(0, openIndex).trim();
	if (!tool) return undefined;
	const specifier = trimmed.slice(openIndex + 1, -1);
	return { tool, specifier };
}

export function normalizeSpecifier(specifier?: string): string | undefined {
	if (!specifier) return undefined;
	const trimmed = specifier.trim();
	if (!trimmed || trimmed === "*") return undefined;
	return trimmed;
}

export function parseArgsMatch(
	tool: string,
	specifier: string | undefined,
	logDebug: DebugLogger,
): unknown | undefined {
	if (!specifier) return undefined;
	if (isKnownTool(tool)) return undefined;
	const trimmed = specifier.trim();
	const candidate = trimmed.startsWith("args:") ? trimmed.slice(5).trim() : trimmed;
	if (!candidate) return undefined;
	if (!candidate.startsWith("{") && !candidate.startsWith("[")) return undefined;
	try {
		return JSON.parse(candidate) as unknown;
	} catch (error) {
		logDebug(`Failed to parse args matcher for ${tool}: ${candidate}`);
		return undefined;
	}
}

export function isKnownTool(tool: string): boolean {
	return tool === "bash" || PATH_TOOLS.has(tool);
}

export function normalizeToolName(name: string): string {
	return name.trim().toLowerCase();
}

export function formatRuleLabel(rule: { raw: string; source: PermissionSource }): string {
	return `${rule.raw} (${rule.source})`;
}

export function matchesPermissionRule(
	rule: PermissionRule,
	toolCall: ToolCallSummary,
	cwd: string,
): boolean {
	return matchesToolRule(rule, toolCall, cwd);
}

export function matchesPolicyOverride(
	rule: PolicyOverrideRule,
	toolCall: ToolCallSummary,
	cwd: string,
): boolean {
	return matchesToolRule(rule, toolCall, cwd);
}

export function getBashCommand(args: Record<string, unknown>): string | undefined {
	return getStringArg(args, "command") ?? getStringArg(args, "cmd");
}

export function getToolPath(args: Record<string, unknown>): string | undefined {
	return getStringArg(args, "path");
}

export function getStringArg(args: Record<string, unknown>, key: string): string | undefined {
	const value = args[key];
	return typeof value === "string" ? value : undefined;
}

function matchesToolRule(
	rule: { tool: string; specifier?: string; argsMatch?: unknown; settingsDir: string },
	toolCall: ToolCallSummary,
	cwd: string,
): boolean {
	const toolName = normalizeToolName(toolCall.name);
	if (toolName !== rule.tool) return false;
	if (!rule.specifier) return true;

	if (rule.tool === "bash") {
		const command = getBashCommand(toolCall.args);
		return command ? matchBashPattern(rule.specifier, command) : false;
	}

	if (PATH_TOOLS.has(rule.tool)) {
		const pathValue = getToolPath(toolCall.args);
		return pathValue ? matchPathPattern(rule.specifier, pathValue, rule.settingsDir, cwd) : false;
	}

	return matchArgs(rule, toolCall.args);
}

function matchBashPattern(pattern: string, command: string): boolean {
	const normalized = normalizeBashPattern(pattern);
	const regex = wildcardToRegExp(normalized);
	return regex.test(command);
}

function normalizeBashPattern(pattern: string): string {
	const trimmed = pattern.trim();
	if (trimmed.endsWith(":*")) {
		return `${trimmed.slice(0, -2)} *`;
	}
	return trimmed;
}

function wildcardToRegExp(pattern: string): RegExp {
	const escaped = pattern.replace(/[.*+?^${}()|[\]\\]/g, "\\$&").replace(/\\\*/g, ".*");
	return new RegExp(`^${escaped}$`);
}

function matchPathPattern(
	pattern: string,
	targetPath: string,
	settingsDir: string,
	cwd: string,
): boolean {
	const normalized = normalizePathPattern(pattern, settingsDir, cwd);
	if (!normalized) return false;
	const { pattern: normalizedPattern, baseDir } = normalized;
	const absoluteTarget = isAbsolute(targetPath) ? resolve(targetPath) : resolve(cwd, targetPath);
	const relativePath = relative(baseDir, absoluteTarget);
	if (relativePath.startsWith("..") && baseDir !== "/") {
		return false;
	}
	const matchPath = toPosixPath(relativePath);
	const ig = ignore();
	ig.add(normalizedPattern);
	return ig.ignores(matchPath);
}

function normalizePathPattern(
	pattern: string,
	settingsDir: string,
	cwd: string,
): { pattern: string; baseDir: string } | undefined {
	const trimmed = pattern.trim();
	if (!trimmed) return undefined;
	if (trimmed.startsWith("//")) {
		return { pattern: toPosixPath(`/${trimmed.slice(2)}`), baseDir: "/" };
	}
	if (trimmed.startsWith("~/")) {
		const absolute = resolve(homedir(), trimmed.slice(2));
		return { pattern: toPosixPath(absolute), baseDir: "/" };
	}
	if (trimmed.startsWith("/")) {
		return { pattern: toPosixPath(trimmed), baseDir: settingsDir };
	}
	if (trimmed.startsWith("./")) {
		return { pattern: toPosixPath(trimmed.slice(2)), baseDir: cwd };
	}
	return { pattern: toPosixPath(trimmed), baseDir: cwd };
}

function matchArgs(rule: { specifier?: string; argsMatch?: unknown }, args: Record<string, unknown>): boolean {
	if (!rule.specifier) return true;
	if (rule.argsMatch === undefined) return false;
	return deepEqual(rule.argsMatch, args);
}
