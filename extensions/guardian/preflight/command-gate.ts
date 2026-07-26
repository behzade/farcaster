import { existsSync, readFileSync, realpathSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import type { ExtensionContext, ToolCallEvent } from "@earendil-works/pi-coding-agent";
import { evaluateCommand, overallDecision, type CommandPolicy } from "./command-policy.js";

export type GateDecision =
	| { action: "allow" }
	| { action: "review"; reason: string; sandboxAllowWrite?: string[] }
	| { action: "block"; reason: string };

const sensitiveRoots = [".ssh", ".aws", ".gnupg"];
const sensitiveFiles = [/(?:^|\/)\.env(?:\..*)?$/, /\.pem$/, /\.key$/];

export function loadCommandPolicy(): CommandPolicy {
	const path = join(getAgentDir(), "extensions", "policy.json");
	try {
		return JSON.parse(readFileSync(path, "utf8")) as CommandPolicy;
	} catch {
		return { defaultDecision: "prompt", rules: [] };
	}
}

export function gateToolCall(
	event: ToolCallEvent,
	ctx: ExtensionContext,
	policy: CommandPolicy,
): GateDecision {
	if (event.toolName === "bash") {
		const command = (event.input as { command?: unknown }).command;
		if (typeof command !== "string") return { action: "review", reason: "Shell command is missing" };
		const matches = evaluateCommand(command, policy);
		const match = overallDecision(matches);
		if (match?.decision === "forbid") {
			return {
				action: "block",
				reason: `Command forbidden by policy rule ${match.ruleId}: ${match.reason}`,
			};
		}
		const writeInspection = inspectShellWrites(matches.map((entry) => entry.command.argv), ctx.cwd);
		if (writeInspection.protectedPath) {
			return {
				action: "block",
				reason: `Writing protected credential path is blocked: ${writeInspection.protectedPath}`,
			};
		}
		if (writeInspection.outsidePaths.length > 0) {
			return {
				action: "review",
				reason: `Command writes outside the project: ${writeInspection.outsidePaths.join(", ")}`,
				sandboxAllowWrite: writeInspection.outsidePaths,
			};
		}
		if (!match || match.decision === "allow") return { action: "allow" };
		return { action: "review", reason: `Command matched review rule ${match.ruleId}` };
	}

	if (event.toolName === "mcp_enable") {
		return { action: "review", reason: "MCP access requires review" };
	}

	if (!(["read", "write", "edit"] as string[]).includes(event.toolName)) {
		return { action: "allow" };
	}

	const paths = toolPaths(event.input, ctx.cwd);
	if (!paths) return { action: "review", reason: "File path is missing" };
	const path = paths.lexical;
	const variants = paths.actual === paths.lexical ? [paths.lexical] : [paths.lexical, paths.actual];
	const home = canonicalize(homedir());
	const cwd = canonicalize(ctx.cwd);
	const sensitive =
		variants.some((candidate) =>
			sensitiveRoots.some((name) => isInside(canonicalize(resolve(home, name)), candidate)),
		) ||
		variants.some((candidate) => {
			const relativePath = relative(cwd, candidate).split(sep).join("/");
			return sensitiveFiles.some((pattern) => pattern.test(relativePath));
		});

	if (event.toolName === "read" && sensitive) {
		return { action: "block", reason: `Reading protected credential path is blocked: ${path}` };
	}
	(event.input as { path: string }).path = paths.actual;
	if (event.toolName === "read") return { action: "allow" };
	if (sensitive) {
		return { action: "block", reason: `Writing protected credential path is blocked: ${path}` };
	}
	if (!isInside(cwd, paths.actual)) {
		return { action: "review", reason: `File write is outside the project: ${path}` };
	}
	return { action: "allow" };
}

function toolPaths(input: unknown, cwd: string): { lexical: string; actual: string } | undefined {
	if (!input || typeof input !== "object" || !("path" in input)) return undefined;
	const value = (input as { path?: unknown }).path;
	if (typeof value !== "string") return undefined;
	const stripped = value.startsWith("@") ? value.slice(1) : value;
	let expanded: string;
	try {
		expanded =
			stripped === "~"
				? homedir()
				: stripped.startsWith("~/")
					? join(homedir(), stripped.slice(2))
					: /^file:\/\//.test(stripped)
						? fileURLToPath(stripped)
						: stripped;
	} catch {
		return undefined;
	}
	const lexical = resolve(isAbsolute(expanded) ? expanded : resolve(cwd, expanded));
	return { lexical, actual: canonicalize(lexical) };
}

function inspectShellWrites(
	commands: string[][],
	cwd: string,
): { outsidePaths: string[]; protectedPath?: string } {
	const root = canonicalize(cwd);
	const home = canonicalize(homedir());
	const outsidePaths = new Set<string>();

	for (const argv of commands) {
		for (const candidate of shellWriteCandidates(argv)) {
			const lexicalPath = resolveLiteralPath(candidate, cwd);
			if (!lexicalPath) continue;
			const actualPath = canonicalize(lexicalPath);
			const variants = actualPath === lexicalPath ? [lexicalPath] : [lexicalPath, actualPath];
			const sensitive =
				variants.some((path) =>
					sensitiveRoots.some((name) => isInside(canonicalize(resolve(home, name)), path)),
				) ||
				variants.some((path) => {
					const relativePath = relative(root, path).split(sep).join("/");
					return sensitiveFiles.some((pattern) => pattern.test(relativePath));
				});
			if (sensitive) return { outsidePaths: [], protectedPath: lexicalPath };
			for (const path of variants) {
				if (!isInside(root, path)) outsidePaths.add(path);
			}
		}
	}

	return { outsidePaths: [...outsidePaths] };
}

function shellWriteCandidates(argv: string[]): string[] {
	if (argv.length === 0) return [];
	const executable = argv[0]?.split("/").at(-1) ?? "";
	const positional = argv.slice(1).filter((arg) => arg !== "--" && !arg.startsWith("-"));
	const redirections: string[] = [];
	for (let index = 0; index < argv.length; index += 1) {
		const arg = argv[index] ?? "";
		if (/^\d*>>?$/.test(arg)) {
			const next = argv[index + 1];
			if (next) redirections.push(next);
			continue;
		}
		const attached = arg.match(/^\d*>>?(.+)$/)?.[1];
		if (attached) redirections.push(attached);
	}

	let commandPaths: string[] = [];
	if (["touch", "mkdir", "mkfifo", "truncate", "rm", "rmdir", "tee"].includes(executable)) {
		commandPaths = positional;
	} else if (["cp", "install", "ln"].includes(executable)) {
		commandPaths = positional.slice(-1);
	} else if (executable === "mv") {
		commandPaths = positional;
	} else if (["chmod", "chown", "chgrp"].includes(executable)) {
		commandPaths = positional.slice(1);
	}
	return [...commandPaths, ...redirections];
}

function resolveLiteralPath(value: string, cwd: string): string | undefined {
	if (!value || value === "-") return undefined;
	const expanded =
		value === "~" || value === "$HOME" || value === "${HOME}"
			? homedir()
			: value.startsWith("~/")
				? join(homedir(), value.slice(2))
				: value.startsWith("$HOME/")
					? join(homedir(), value.slice(6))
					: value.startsWith("${HOME}/")
						? join(homedir(), value.slice(8))
						: value;
	if (/[$*?[\]{}()`]/.test(expanded)) return undefined;
	return resolve(cwd, expanded);
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

function getAgentDir(): string {
	return process.env.PI_CODING_AGENT_DIR || join(homedir(), ".pi", "agent");
}
