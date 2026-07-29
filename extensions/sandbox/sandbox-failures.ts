const FAILURE_KEYWORDS = [
	"operation not permitted",
	"permission denied",
	"read-only file system",
	"failed to write file",
] as const;

function unquote(value: string): string {
	if (
		(value.startsWith('"') && value.endsWith('"')) ||
		(value.startsWith("'") && value.endsWith("'"))
	) {
		return value.slice(1, -1);
	}
	return value;
}

export interface ParsedFilesystemFailure {
	path: string;
	targetType?: "folder";
}

/** Extract exact absolute paths only from lines that report a likely access failure. */
export function parseFilesystemFailures(output: string): ParsedFilesystemFailure[] {
	const failures = new Map<string, ParsedFilesystemFailure>();
	for (const line of output.split("\n")) {
		const lower = line.toLowerCase();
		const keyword = FAILURE_KEYWORDS.find((entry) => lower.includes(entry));
		if (!keyword) continue;
		const keywordIndex = lower.indexOf(keyword);
		const targetType =
			lower.includes("cannot create directory") || lower.includes("cannot mkdir")
				? "folder"
				: undefined;
		const record = (path: string) =>
			failures.set(path, { path, ...(targetType ? { targetType } : {}) });

		// GNU tools often wrap the exact target in locale quotes:
		// `mkdir: cannot create directory ‘/absolute/path’: Read-only file system`.
		const quotedBeforeFailure = line
			.slice(0, keywordIndex)
			.match(/["'‘“](\/[^"'’”]+)["'’”]/g)
			?.at(-1)
			?.match(/["'‘“](\/[^"'’”]+)["'’”]/)?.[1];
		if (quotedBeforeFailure) {
			record(quotedBeforeFailure);
			continue;
		}

		// Common shell/tool form: `tool: /absolute/path: Permission denied`.
		const before = line.slice(0, keywordIndex).replace(/[,:\s]+$/, "");
		const colonPath = before.match(/:\s+(["']?\/.*)$/)?.[1];
		if (colonPath) {
			const path = unquote(colonPath.trim());
			if (path.startsWith("/")) record(path);
			continue;
		}

		// Common runtime form: `EACCES: permission denied, open '/absolute/path'`.
		const after = line.slice(keywordIndex + keyword.length);
		const quotedPath = after.match(/["'](\/[^"']+)["']/)?.[1];
		if (quotedPath) record(quotedPath);
	}
	return [...failures.values()];
}

export function parseFilesystemFailurePaths(output: string): string[] {
	return parseFilesystemFailures(output).map((failure) => failure.path);
}
