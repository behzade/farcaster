import { homedir } from "node:os";
import { dirname, relative, resolve, sep } from "node:path";
import type { BrokerDenial } from "./broker-client.ts";

const MAX_EXAMPLES = 3;

interface DenialGroup {
	label: string;
	count: number;
	paths: string[];
	examples: string[];
}

export function formatDenialSummary(
	denials: readonly BrokerDenial[],
	complete: boolean,
): string | undefined {
	const groups = new Map<string, DenialGroup>();
	let retained = 0;
	let hasCache = false;
	let hasGeneralAccess = false;

	for (const denial of denials) {
		const access = denial.operation.startsWith("file-write")
			? "write"
			: denial.operation.startsWith("file-read")
				? "read"
				: undefined;
		if (access) {
			if (!denial.path || isInside("/dev", resolve(denial.path))) continue;
			const cache = hostCacheCategory(denial.path);
			const key = cache ? `cache:${cache}` : `filesystem:${access}`;
			const group = groups.get(key) ?? {
				label: cache ? `host development cache (${cache})` : `${access} access`,
				count: 0,
				paths: [],
				examples: [],
			};
			group.count += 1;
			if (!group.paths.includes(denial.path)) group.paths.push(denial.path);
			if (!group.examples.includes(denial.path)) group.examples.push(denial.path);
			groups.set(key, group);
			hasCache ||= cache !== undefined;
			hasGeneralAccess ||= cache === undefined;
			retained += 1;
			continue;
		}
		if (denial.operation.startsWith("network")) {
			const key = "network";
			const group = groups.get(key) ?? {
				label: "network access",
				count: 0,
				paths: [],
				examples: [],
			};
			group.count += 1;
			if (denial.path && !group.examples.includes(denial.path)) {
				group.examples.push(denial.path);
			} else if (denial.process) {
				const example = `process ${denial.process}`;
				if (!group.examples.includes(example)) group.examples.push(example);
			}
			groups.set(key, group);
			hasGeneralAccess = true;
			retained += 1;
		}
	}
	if (retained === 0) return undefined;

	let remainingExamples = MAX_EXAMPLES;
	const lines = [
		`Sandbox reported ${retained} denial hint${retained === 1 ? "" : "s"}${complete ? "" : " (best-effort diagnostics)"}.`,
	];
	for (const group of groups.values()) {
		const location = group.paths.length > 0 ? ` under ${commonCategoryRoot(group.paths)}` : "";
		lines.push(`- ${group.label}: ${group.count}${location}`);
		const examples = group.examples.slice(0, remainingExamples);
		for (const example of examples) lines.push(`  example: ${example}`);
		remainingExamples -= examples.length;
	}
	if (hasCache) {
		lines.push(
			"Use request_access with a development_cache environment mapping to route the tool into the shared managed cache; do not grant its host cache.",
		);
	}
	if (hasGeneralAccess) {
		lines.push(
			"Use request_access for the smallest portable file/tree, exact network host, or network_local right, then explicitly rerun the command.",
		);
	}
	lines.push("No command was retried.");
	return `\n${lines.join("\n")}\n`;
}

function commonCategoryRoot(paths: readonly string[]): string {
	if (paths.length === 0) return "/";
	let parts = dirname(paths[0] ?? "/").split(sep);
	for (const path of paths.slice(1)) {
		const candidate = dirname(path).split(sep);
		let index = 0;
		while (index < parts.length && parts[index] === candidate[index]) index += 1;
		parts = parts.slice(0, index);
	}
	const joined = parts.join(sep);
	return joined || sep;
}

function hostCacheCategory(path: string): string | undefined {
	const home = homedir();
	const categories: Array<[string, string]> = [
		[resolve(home, ".cargo"), "Cargo"],
		[resolve(home, ".npm"), "npm"],
		[resolve(home, ".gradle"), "Gradle"],
		[resolve(home, ".bun", "install", "cache"), "Bun"],
		[resolve(home, "go", "pkg", "mod"), "Go modules"],
		[resolve(home, ".cache"), "XDG caches"],
		[resolve(home, "Library", "Caches"), "macOS caches"],
	];
	return categories.find(([root]) => isInside(root, path))?.[1];
}

function isInside(root: string, path: string): boolean {
	const rel = relative(root, path);
	return rel === "" || (rel !== ".." && !rel.startsWith(`..${sep}`));
}
