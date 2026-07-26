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

/** Extract exact absolute paths only from lines that report a likely access failure. */
export function parseFilesystemFailurePaths(output: string): string[] {
	const paths = new Set<string>();
	for (const line of output.split("\n")) {
		const lower = line.toLowerCase();
		const keyword = FAILURE_KEYWORDS.find((entry) => lower.includes(entry));
		if (!keyword) continue;
		const keywordIndex = lower.indexOf(keyword);

		// Common shell/tool form: `tool: /absolute/path: Permission denied`.
		const before = line.slice(0, keywordIndex).replace(/[,:\s]+$/, "");
		const colonPath = before.match(/:\s+(["']?\/.*)$/)?.[1];
		if (colonPath) {
			const path = unquote(colonPath.trim());
			if (path.startsWith("/")) paths.add(path);
			continue;
		}

		// Common runtime form: `EACCES: permission denied, open '/absolute/path'`.
		const after = line.slice(keywordIndex + keyword.length);
		const quotedPath = after.match(/["'](\/[^"']+)["']/)?.[1];
		if (quotedPath) paths.add(quotedPath);
	}
	return [...paths];
}
