import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import type { Api, Model } from "@earendil-works/pi-ai";
import type { KeyId } from "@earendil-works/pi-tui";
import { expandTilde } from "./utils/path.js";
import type { ApprovalMode, ModelRef, PreflightConfig, SessionConfigEntryData } from "./types.js";

export const SESSION_ENTRY_TYPE = "bo-pi-config";

export const DEFAULT_CONFIG: PreflightConfig = {
	contextMessages: 1,
	explainKey: "ctrl+e",
	ruleSuggestionKey: "ctrl+n",
	model: { provider: "openai-codex", id: "gpt-5.6-terra" },
	policyModel: "current",
	reasoning: "low",
	approvalMode: "auto",
	repeatThreshold: 2,
	debug: false,
};

export function loadPersistentConfig(): PreflightConfig {
	const persistent = readConfigFile(getConfigFilePath());
	const managed = readConfigFile(getManagedConfigFilePath());
	return { ...DEFAULT_CONFIG, ...persistent, ...managed };
}

export function savePersistentConfig(config: PreflightConfig): void {
	const filePath = getConfigFilePath();
	mkdirSync(dirname(filePath), { recursive: true });
	writeFileSync(filePath, `${JSON.stringify(config, null, 2)}\n`);
}

export function loadSessionOverrides(ctx: ExtensionContext): Partial<PreflightConfig> {
	const entries = ctx.sessionManager.getBranch();
	for (let i = entries.length - 1; i >= 0; i--) {
		const entry = entries[i];
		if (!isCustomEntry(entry)) continue;
		if (entry.customType !== SESSION_ENTRY_TYPE) continue;
		const data = entry.data as SessionConfigEntryData | undefined;
		if (!data || data.config === null) return {};
		return parseConfig(data.config);
	}
	return {};
}

export function parseConfig(value: unknown): Partial<PreflightConfig> {
	if (!value || typeof value !== "object") return {};

	const record = value as Record<string, unknown>;
	const config: Partial<PreflightConfig> = {};

	if (typeof record.enabled === "boolean" && record.enabled === false) {
		config.approvalMode = "off";
	}

	if (typeof record.contextMessages === "number" && Number.isFinite(record.contextMessages)) {
		const normalized = Math.floor(record.contextMessages);
		if (normalized < 0) {
			config.contextMessages = -1;
		} else if (normalized === 0) {
			config.contextMessages = 1;
		} else {
			config.contextMessages = normalized;
		}
	}

	const explainKey = parseExplainKey(record.explainKey);
	if (explainKey) {
		config.explainKey = explainKey;
	}

	const ruleSuggestionKey = parseExplainKey(record.ruleSuggestionKey);
	if (ruleSuggestionKey) {
		config.ruleSuggestionKey = ruleSuggestionKey;
	}

	if (record.model === "current") {
		config.model = "current";
	} else if (isModelRef(record.model)) {
		config.model = record.model;
	}

	if (record.policyModel === "current") {
		config.policyModel = "current";
	} else if (isModelRef(record.policyModel)) {
		config.policyModel = record.policyModel;
	}

	if (
		record.reasoning === "minimal" ||
		record.reasoning === "low" ||
		record.reasoning === "medium" ||
		record.reasoning === "high" ||
		record.reasoning === "xhigh" ||
		record.reasoning === "max"
	) {
		config.reasoning = record.reasoning;
	}

	if (typeof record.approvalMode === "string") {
		const parsed = parseApprovalMode([record.approvalMode]);
		if (parsed) {
			config.approvalMode = parsed;
		}
	} else if (typeof record.approveDestructiveOnly === "boolean") {
		config.approvalMode = record.approveDestructiveOnly ? "destructive" : "all";
	}

	if (
		typeof record.repeatThreshold === "number" &&
		Number.isFinite(record.repeatThreshold) &&
		record.repeatThreshold >= 2
	) {
		config.repeatThreshold = Math.floor(record.repeatThreshold);
	}

	if (typeof record.debug === "boolean") {
		config.debug = record.debug;
	}

	return config;
}

export function parseExplainKey(value: unknown): KeyId | KeyId[] | undefined {
	if (typeof value === "string") return value as KeyId;
	if (Array.isArray(value)) {
		const keys = value.filter((item): item is KeyId => typeof item === "string");
		return keys.length > 0 ? keys : undefined;
	}
	return undefined;
}

export function parseModelRef(parts: string[]): PreflightConfig["model"] | undefined {
	if (parts.length === 0) return undefined;
	const joined = parts.join(" ").trim();
	if (!joined) return undefined;
	if (joined === "current") return "current";

	const [provider, id] = joined.split("/");
	if (!provider || !id) return undefined;
	return { provider, id };
}

export function parseApprovalMode(parts: string[]): ApprovalMode | undefined {
	const joined = parts.join(" ").trim().toLowerCase();
	if (!joined) return undefined;
	if (joined === "auto" || joined === "approve for me" || joined === "approve-for-me") return "auto";
	if (joined === "all" || joined === "all tools" || joined === "all-tools") return "all";
	if (
		joined === "destructive" ||
		joined === "destructive only" ||
		joined === "destructive-only"
	) {
		return "destructive";
	}
	if (joined === "off") return "off";
	return undefined;
}

export function parseContextValue(value: string): number | undefined {
	const trimmed = value.trim().toLowerCase();
	if (!trimmed) return undefined;
	if (trimmed === "full") return -1;
	const numberValue = Number(trimmed);
	if (!Number.isFinite(numberValue)) return undefined;
	if (numberValue < 1) return undefined;
	return Math.floor(numberValue);
}

export function formatApprovalMode(mode: ApprovalMode): string {
	switch (mode) {
		case "auto":
			return "approve for me";
		case "off":
			return "off";
		case "destructive":
			return "destructive only";
		case "all":
		default:
			return "all tools";
	}
}

export function formatContextMessages(limit: number): string {
	if (limit < 0) return "full";
	return String(limit <= 0 ? 1 : limit);
}

export function formatContextLabel(limit: number): string {
	if (limit < 0) return "full";
	const safeLimit = limit <= 0 ? 1 : limit;
	return `last ${safeLimit}`;
}

export function formatModelSetting(
	modelSetting: PreflightConfig["model"],
	currentModel?: Model<Api>,
): string {
	if (modelSetting === "current") {
		if (currentModel) {
			return `current (${currentModel.provider}/${currentModel.id})`;
		}
		return "current";
	}
	return `${modelSetting.provider}/${modelSetting.id}`;
}

function isModelRef(value: unknown): value is ModelRef {
	if (!value || typeof value !== "object") return false;
	const record = value as Record<string, unknown>;
	return typeof record.provider === "string" && typeof record.id === "string";
}

function isCustomEntry(entry: { type: string; customType?: string; data?: unknown }): entry is {
	type: "custom";
	customType: string;
	data?: unknown;
} {
	return entry.type === "custom" && typeof entry.customType === "string";
}

function getConfigFilePath(): string {
	return join(getAgentDir(), "extensions", "bo-pi", "preflight.json");
}

function getManagedConfigFilePath(): string {
	return join(getAgentDir(), "extensions", "guardian.json");
}

function readConfigFile(filePath: string): Partial<PreflightConfig> {
	if (!existsSync(filePath)) return {};
	try {
		return parseConfig(JSON.parse(readFileSync(filePath, "utf-8")) as unknown);
	} catch {
		return {};
	}
}

function getAgentDir(): string {
	const envDir = process.env.PI_CODING_AGENT_DIR;
	if (envDir) {
		return expandTilde(envDir);
	}
	return join(homedir(), ".pi", "agent");
}
