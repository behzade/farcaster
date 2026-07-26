import { createHash, randomUUID } from "node:crypto";
import {
	existsSync,
	mkdirSync,
	readFileSync,
	realpathSync,
	renameSync,
	statSync,
	writeFileSync,
} from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import type { DebugLogger, ToolCallSummary } from "./types.js";
import { stableStringify } from "./utils/json.js";
import { getBashCommand } from "./permissions/matching.js";
import { parseShellCommands } from "./command-policy.js";

const PROJECT_RULES_PATH = join(".pi", "preflight", "settings.local.json");

interface StoredGuardianRule {
	fingerprint: string;
	label: string;
	createdAt: string;
}

interface GuardianSettings {
	allow?: StoredGuardianRule[];
}

export interface ActionFingerprint {
	fingerprint: string;
	label: string;
}

export class RepeatTracker {
	private readonly counts = new Map<string, number>();

	clear(): void {
		this.counts.clear();
	}

	recordAllowed(fingerprint: string): number {
		const next = (this.counts.get(fingerprint) ?? 0) + 1;
		this.counts.set(fingerprint, next);
		return next;
	}

	reset(fingerprint: string): void {
		this.counts.delete(fingerprint);
	}
}

export function buildActionFingerprint(toolCall: ToolCallSummary, cwd: string): ActionFingerprint {
	const normalizedArgs = normalizeArgs(toolCall, cwd);
	const payload = stableStringify({
		version: 1,
		cwd: canonicalize(cwd),
		tool: toolCall.name.trim().toLowerCase(),
		args: normalizedArgs,
	});
	return {
		fingerprint: createHash("sha256").update(payload).digest("hex"),
		label: buildLabel(toolCall),
	};
}

export function hasProjectGuardianRule(
	cwd: string,
	action: ActionFingerprint,
	logDebug: DebugLogger,
	projectTrusted: boolean,
): boolean {
	if (!projectTrusted) return false;
	const settings = readSettings(cwd, logDebug);
	return (settings.guardian?.allow ?? []).some((entry) => entry.fingerprint === action.fingerprint);
}

export function persistProjectGuardianRule(
	cwd: string,
	action: ActionFingerprint,
	logDebug: DebugLogger,
	projectTrusted: boolean,
): boolean {
	if (!projectTrusted) {
		throw new Error("Refusing to save a Guardian rule for an untrusted project");
	}
	const filePath = getProjectRulesPath(cwd);
	const settings = readSettings(cwd, logDebug, true);
	const guardian = normalizeGuardianSettings(settings.guardian);
	if ((guardian.allow ?? []).some((entry) => entry.fingerprint === action.fingerprint)) return false;

	const nextRule: StoredGuardianRule = {
		fingerprint: action.fingerprint,
		label: action.label,
		createdAt: new Date().toISOString(),
	};
	const next = {
		...settings,
		version: typeof settings.version === "number" ? settings.version : 1,
		guardian: {
			...guardian,
			allow: [nextRule, ...(guardian.allow ?? [])],
		},
	};

	mkdirSync(dirname(filePath), { recursive: true });
	const temporaryPath = `${filePath}.tmp-${process.pid}-${randomUUID()}`;
	writeFileSync(temporaryPath, `${JSON.stringify(next, null, 2)}\n`, { mode: 0o600 });
	renameSync(temporaryPath, filePath);
	logDebug(`Saved guardian allow rule to ${filePath}: ${action.label}`);
	return true;
}

function readSettings(cwd: string, logDebug: DebugLogger): Record<string, unknown> & {
	version?: number;
	guardian?: GuardianSettings;
};
function readSettings(
	cwd: string,
	logDebug: DebugLogger,
	strict: boolean,
): Record<string, unknown> & {
	version?: number;
	guardian?: GuardianSettings;
};
function readSettings(
	cwd: string,
	logDebug: DebugLogger,
	strict = false,
): Record<string, unknown> & {
	version?: number;
	guardian?: GuardianSettings;
} {
	const filePath = getProjectRulesPath(cwd);
	if (!existsSync(filePath)) return {};
	try {
		const parsed = JSON.parse(readFileSync(filePath, "utf8")) as unknown;
		if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
			throw new Error("settings must contain a JSON object");
		}
		return parsed as Record<string, unknown> & { version?: number; guardian?: GuardianSettings };
	} catch (error) {
		logDebug(`Could not read guardian rules from ${filePath}: ${String(error)}`);
		if (strict) {
			throw new Error(`Refusing to replace invalid Guardian settings at ${filePath}`);
		}
		return {};
	}
}

function normalizeGuardianSettings(value: unknown): GuardianSettings {
	if (!value || typeof value !== "object" || Array.isArray(value)) return {};
	const record = value as { allow?: unknown };
	const allow = Array.isArray(record.allow)
		? record.allow.filter((entry): entry is StoredGuardianRule => {
				if (!entry || typeof entry !== "object") return false;
				const candidate = entry as Partial<StoredGuardianRule>;
				return (
					typeof candidate.fingerprint === "string" &&
					typeof candidate.label === "string" &&
					typeof candidate.createdAt === "string"
				);
			})
		: [];
	return { allow };
}

function normalizeArgs(toolCall: ToolCallSummary, cwd: string): unknown {
	const args = { ...toolCall.args };
	if (typeof args.path === "string") {
		args.path = canonicalize(isAbsolute(args.path) ? args.path : resolve(cwd, args.path));
	}
	const command = getBashCommand(args);
	if (!command) return args;

	const normalizedCommand = command.trim();
	return {
		...args,
		command: normalizedCommand,
		script: resolveScriptIdentity(normalizedCommand, cwd),
	};
}

function resolveScriptIdentity(
	command: string,
	cwd: string,
): { path: string; sha256: string } | undefined {
	const commands = parseShellCommands(command);
	if (commands.length !== 1) return undefined;
	const argv = commands[0]?.argv ?? [];
	if (argv.length === 0) return undefined;

	let candidate: string | undefined;
	const executable = argv[0];
	if (["bash", "sh", "zsh", "python", "python3", "node", "bun"].includes(executable)) {
		candidate = argv.find((arg, index) => index > 0 && !arg.startsWith("-"));
	} else if (executable.includes("/") || /\.(?:sh|py|js|mjs|cjs|ts)$/.test(executable)) {
		candidate = executable;
	}
	if (!candidate) return undefined;

	const path = canonicalize(isAbsolute(candidate) ? candidate : resolve(cwd, candidate));
	if (!isInside(canonicalize(cwd), path) || !existsSync(path) || !statSync(path).isFile()) return undefined;
	return {
		path,
		sha256: createHash("sha256").update(readFileSync(path)).digest("hex"),
	};
}

function buildLabel(toolCall: ToolCallSummary): string {
	const command = getBashCommand(toolCall.args);
	if (command) return command.trim().slice(0, 240);
	if (typeof toolCall.args.path === "string") {
		return `${toolCall.name} ${toolCall.args.path}`.slice(0, 240);
	}
	return `${toolCall.name} ${stableStringify(toolCall.args)}`.slice(0, 240);
}

function canonicalize(path: string): string {
	if (existsSync(path)) return realpathSync.native(path);
	const parent = dirname(path);
	if (parent === path) return path;
	return resolve(canonicalize(parent), path.slice(parent.length + (parent.endsWith(sep) ? 0 : 1)));
}

function isInside(root: string, path: string): boolean {
	const rel = relative(root, path);
	return rel === "" || (!rel.startsWith(`..${sep}`) && rel !== "..");
}

function getProjectRulesPath(cwd: string): string {
	return join(findProjectRoot(cwd), PROJECT_RULES_PATH);
}

function findProjectRoot(cwd: string): string {
	let current = canonicalize(cwd);
	while (true) {
		if (existsSync(join(current, ".jj")) || existsSync(join(current, ".git"))) return current;
		const parent = dirname(current);
		if (parent === current) return canonicalize(cwd);
		current = parent;
	}
}
