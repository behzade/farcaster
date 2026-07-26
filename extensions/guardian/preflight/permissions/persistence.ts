import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, resolve } from "node:path";
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import type {
	DebugLogger,
	PermissionDecision,
	PermissionSettingsFile,
	ToolCallSummary,
} from "../types.js";
import { notify } from "../ui.js";
import { stableStringify } from "../utils/json.js";
import { toPosixPath } from "../utils/path.js";
import {
	PATH_TOOLS,
	extractPermissionList,
	getBashCommand,
	getToolPath,
	normalizeToolName,
	parseToolPattern,
} from "./matching.js";

const WORKSPACE_PERMISSIONS_PATH = join(".pi", "preflight", "settings.local.json");
const GLOBAL_PERMISSIONS_PATH = join(".pi", "preflight", "settings.json");

export function persistWorkspaceRule(
	toolCall: ToolCallSummary,
	decision: PermissionDecision,
	ctx: ExtensionContext,
	logDebug: DebugLogger,
): void {
	if (decision !== "allow" && decision !== "deny") {
		notify(ctx, "Could not save rule for this tool call.");
		logDebug(`Unsupported rule type for ${toolCall.name}: ${decision}.`);
		return;
	}
	const ruleKind: PermissionDecision = decision;
	const rule = buildRuleForToolCall(toolCall, ctx.cwd);
	if (!rule) {
		notify(ctx, "Could not save rule for this tool call.");
		logDebug(`Failed to build rule for ${toolCall.name}.`);
		return;
	}

	const filePath = getWorkspacePermissionsPath(ctx.cwd);
	const saved = addRuleToPermissionsFile(filePath, ruleKind, rule, ctx, logDebug);
	if (saved) {
		logDebug(`Saved ${ruleKind} rule to ${filePath}: ${rule}`);
	}
}

export function persistPolicyOverride(
	toolCall: ToolCallSummary,
	ctx: ExtensionContext,
	logDebug: DebugLogger,
): void {
	const rule = buildRuleForToolCall(toolCall, ctx.cwd);
	if (!rule) {
		notify(ctx, "Could not save policy override for this tool call.");
		logDebug(`Failed to build policy override for ${toolCall.name}.`);
		return;
	}

	const filePath = getWorkspacePermissionsPath(ctx.cwd);
	const saved = addPolicyOverrideToPermissionsFile(filePath, rule, ctx, logDebug);
	if (saved) {
		logDebug(`Saved policy override to ${filePath}: ${rule}`);
	}
}

export function persistPolicyRule(
	toolCall: ToolCallSummary,
	policy: string,
	ctx: ExtensionContext,
	logDebug: DebugLogger,
): void {
	const trimmedPolicy = policy.trim();
	if (!trimmedPolicy) {
		notify(ctx, "Custom rule was empty.");
		logDebug("Failed to save policy rule: empty rule.");
		return;
	}

	const filePath = getWorkspacePermissionsPath(ctx.cwd);
	const saved = addPolicyRuleToPermissionsFile(filePath, toolCall, trimmedPolicy, ctx, logDebug);
	if (saved) {
		logDebug(`Saved policy rule for ${toolCall.name} to ${filePath}: ${trimmedPolicy}`);
	}
}

function addRuleToPermissionsFile(
	filePath: string,
	kind: PermissionDecision,
	rule: string,
	ctx: ExtensionContext,
	logDebug: DebugLogger,
): boolean {
	const existing = readPermissionsFile(filePath, logDebug) ?? {};
	const normalized = normalizePermissionsRecord(existing.permissions);

	const list = kind === "deny" ? normalized.deny : kind === "ask" ? normalized.ask : normalized.allow;
	if (list.includes(rule)) {
		notify(ctx, `Rule already exists: ${rule}`);
		return false;
	}

	list.unshift(rule);
	const nextPermissions = {
		...normalized.record,
		allow: normalized.allow,
		ask: normalized.ask,
		deny: normalized.deny,
	};
	const nextConfig: PermissionSettingsFile = {
		...existing,
		version: typeof existing.version === "number" ? existing.version : 1,
		permissions: nextPermissions,
	};

	mkdirSync(dirname(filePath), { recursive: true });
	writeFileSync(filePath, `${JSON.stringify(nextConfig, null, 2)}\n`);
	if (ctx.hasUI) {
		ctx.ui.notify(`Saved ${kind} rule: ${rule}`, "info");
	}
	return true;
}

function addPolicyOverrideToPermissionsFile(
	filePath: string,
	rule: string,
	ctx: ExtensionContext,
	logDebug: DebugLogger,
): boolean {
	const existing = readPermissionsFile(filePath, logDebug) ?? {};
	const preflight = normalizePreflightRecord(existing.preflight, undefined, logDebug);
	const overrides = preflight.policyOverrides;

	if (overrides.includes(rule)) {
		notify(ctx, `Policy override already exists: ${rule}`);
		return false;
	}

	overrides.unshift(rule);
	const nextPreflight = {
		...preflight.record,
		policyOverrides: overrides,
		llmRules: preflight.llmRules,
	};
	const nextConfig: PermissionSettingsFile = {
		...existing,
		version: typeof existing.version === "number" ? existing.version : 1,
		preflight: nextPreflight,
	};

	mkdirSync(dirname(filePath), { recursive: true });
	writeFileSync(filePath, `${JSON.stringify(nextConfig, null, 2)}\n`);
	if (ctx.hasUI) {
		ctx.ui.notify(`Saved policy override: ${rule}`, "info");
	}
	return true;
}

function addPolicyRuleToPermissionsFile(
	filePath: string,
	toolCall: ToolCallSummary,
	policy: string,
	ctx: ExtensionContext,
	logDebug: DebugLogger,
): boolean {
	const tool = normalizeToolName(toolCall.name);
	if (!tool) {
		notify(ctx, "Could not save custom rule for this tool.");
		logDebug("Failed to save policy rule: invalid tool name.");
		return false;
	}

	const existing = readPermissionsFile(filePath, logDebug) ?? {};
	const preflight = normalizePreflightRecord(existing.preflight, tool, logDebug);
	const toolRules = preflight.llmRules[tool] ?? [];

	if (toolRules.includes(policy)) {
		notify(ctx, `Policy rule already exists: ${policy}`);
		return false;
	}

	const nextToolRules = [policy, ...toolRules];
	const nextLlmRules = {
		...preflight.llmRules,
		[tool]: nextToolRules,
	};

	const nextPreflight = {
		...preflight.record,
		policyOverrides: preflight.policyOverrides,
		llmRules: nextLlmRules,
	};
	const nextConfig: PermissionSettingsFile = {
		...existing,
		version: typeof existing.version === "number" ? existing.version : 1,
		preflight: nextPreflight,
	};

	mkdirSync(dirname(filePath), { recursive: true });
	writeFileSync(filePath, `${JSON.stringify(nextConfig, null, 2)}\n`);
	if (ctx.hasUI) {
		ctx.ui.notify(`Saved custom rule for ${tool}: ${policy}`, "info");
	}
	return true;
}

function normalizePermissionsRecord(value: Record<string, unknown> | undefined): {
	record: Record<string, unknown>;
	allow: string[];
	ask: string[];
	deny: string[];
} {
	const record = value && typeof value === "object" ? { ...value } : {};
	const allow = extractPermissionList(record.allow);
	const ask = extractPermissionList(record.ask);
	const deny = extractPermissionList(record.deny);
	return { record, allow, ask, deny };
}

function normalizePreflightRecord(
	value: Record<string, unknown> | undefined,
	currentTool: string | undefined,
	logDebug: DebugLogger,
): {
	record: Record<string, unknown>;
	policyOverrides: string[];
	llmRules: Record<string, string[]>;
} {
	const record = value && typeof value === "object" ? { ...value } : {};
	const policyOverrides = extractPermissionList(record.policyOverrides);
	const llmRules = normalizeLlmRules(record.llmRules, currentTool, logDebug);
	return { record, policyOverrides, llmRules };
}

function normalizeLlmRules(
	value: unknown,
	currentTool: string | undefined,
	logDebug: DebugLogger,
): Record<string, string[]> {
	if (!value) return {};

	if (!Array.isArray(value) && typeof value === "object") {
		return normalizeToolScopedLlmRules(value as Record<string, unknown>, logDebug);
	}

	if (!Array.isArray(value)) {
		logDebug("Ignored invalid llmRules value while normalizing persistence state.");
		return {};
	}

	return migrateLegacyLlmRules(value, currentTool, logDebug);
}

function normalizeToolScopedLlmRules(
	value: Record<string, unknown>,
	logDebug: DebugLogger,
): Record<string, string[]> {
	const result: Record<string, string[]> = {};

	for (const [toolName, policiesValue] of Object.entries(value)) {
		const tool = normalizeToolName(toolName);
		if (!tool) {
			logDebug("Ignored llmRules entry with empty tool name.");
			continue;
		}
		const policies = extractPoliciesForTool(policiesValue, logDebug);
		if (policies.length > 0) {
			result[tool] = policies;
		}
	}

	return result;
}

function extractPoliciesForTool(value: unknown, logDebug: DebugLogger): string[] {
	if (typeof value === "string") {
		const trimmed = value.trim();
		return trimmed ? [trimmed] : [];
	}
	if (!Array.isArray(value)) {
		if (value !== undefined) {
			logDebug("Ignored invalid llmRules tool value while normalizing persistence state.");
		}
		return [];
	}

	const result: string[] = [];
	for (const item of value) {
		if (typeof item === "string") {
			const trimmed = item.trim();
			if (trimmed && !result.includes(trimmed)) {
				result.push(trimmed);
			}
			continue;
		}
		if (item && typeof item === "object") {
			const record = item as { policy?: unknown };
			if (typeof record.policy === "string") {
				const trimmed = record.policy.trim();
				if (trimmed && !result.includes(trimmed)) {
					result.push(trimmed);
				}
				continue;
			}
		}
		logDebug("Ignored invalid llmRules policy entry while normalizing persistence state.");
	}

	return result;
}

function migrateLegacyLlmRules(
	value: unknown[],
	currentTool: string | undefined,
	logDebug: DebugLogger,
): Record<string, string[]> {
	const result: Record<string, string[]> = {};

	for (const item of value) {
		if (typeof item === "string") {
			const trimmed = item.trim();
			if (!trimmed) continue;
			if (!currentTool) {
				logDebug("Skipped legacy global policy string during migration: current tool unavailable.");
				continue;
			}
			appendPolicy(result, currentTool, trimmed);
			continue;
		}

		if (!item || typeof item !== "object") {
			logDebug("Ignored invalid legacy llmRules entry while migrating.");
			continue;
		}

		const record = item as { pattern?: unknown; policy?: unknown; tool?: unknown };
		if (typeof record.policy !== "string" || !record.policy.trim()) {
			logDebug("Ignored legacy llmRules object without policy while migrating.");
			continue;
		}
		const policy = record.policy.trim();

		if (typeof record.tool === "string" && record.tool.trim()) {
			appendPolicy(result, normalizeToolName(record.tool), policy);
			continue;
		}

		if (typeof record.pattern === "string" && record.pattern.trim()) {
			const parsed = parseToolPattern(record.pattern);
			if (!parsed) {
				logDebug(`Ignored legacy llmRules pattern during migration: ${record.pattern}`);
				continue;
			}
			appendPolicy(result, normalizeToolName(parsed.tool), policy);
			continue;
		}

		if (!currentTool) {
			logDebug("Skipped legacy policy object during migration: current tool unavailable.");
			continue;
		}
		appendPolicy(result, currentTool, policy);
	}

	return result;
}

function appendPolicy(target: Record<string, string[]>, tool: string, policy: string): void {
	if (!tool) return;
	if (!target[tool]) target[tool] = [];
	if (!target[tool].includes(policy)) {
		target[tool].push(policy);
	}
}

function buildRuleForToolCall(toolCall: ToolCallSummary, cwd: string): string | undefined {
	const toolName = formatRuleToolName(toolCall.name);
	const toolKey = normalizeToolName(toolCall.name);

	if (toolKey === "bash") {
		const command = getBashCommand(toolCall.args);
		return command ? `${toolName}(${command})` : undefined;
	}

	if (PATH_TOOLS.has(toolKey)) {
		const pathValue = getToolPath(toolCall.args);
		if (!pathValue) return undefined;
		return `${toolName}(${formatPathRule(pathValue)})`;
	}

	const argsString = stableStringify(toolCall.args);
	if (!argsString) return undefined;
	return `${toolName}(args:${argsString})`;
}

function formatRuleToolName(name: string): string {
	const normalized = normalizeToolName(name);
	if (normalized === "bash") return "Bash";
	if (normalized === "read") return "Read";
	if (normalized === "edit") return "Edit";
	if (normalized === "write") return "Write";
	return name;
}

function formatPathRule(pathValue: string): string {
	const trimmed = pathValue.trim();
	if (!trimmed) return trimmed;
	if (trimmed.startsWith("~/")) {
		return toPosixPath(trimmed);
	}
	if (isAbsolute(trimmed)) {
		const absolute = resolve(trimmed);
		return `//${toPosixPath(absolute).replace(/^\/+/, "")}`;
	}
	return toPosixPath(trimmed);
}

export function readPermissionsFile(
	filePath: string,
	logDebug: DebugLogger,
): PermissionSettingsFile | undefined {
	if (!existsSync(filePath)) return undefined;
	try {
		const content = readFileSync(filePath, "utf-8");
		const parsed = JSON.parse(content) as unknown;
		if (!parsed || typeof parsed !== "object") return undefined;
		return parsed as PermissionSettingsFile;
	} catch (error) {
		logDebug(`Failed to read permissions file ${filePath}.`);
		return undefined;
	}
}

export function getWorkspacePermissionsPath(cwd: string): string {
	return join(cwd, WORKSPACE_PERMISSIONS_PATH);
}

export function getGlobalPermissionsPath(): string {
	return join(homedir(), GLOBAL_PERMISSIONS_PATH);
}
