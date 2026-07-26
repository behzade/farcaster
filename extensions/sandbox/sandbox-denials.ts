export interface FilesystemDenial {
	access: "read" | "write";
	path: string;
}

const rawDenialPattern = /\bdeny(?:\(\d+\))?\s+file-([a-z-]+)\s+("[^"]+"|\/\S.*)$/i;
// `codex sandbox --log-denials` does not print the raw macOS log entry. It
// prints each parsed denial as `(<process>) <capability>` after the command.
const codexDenialPattern = /^\([^)]+\)\s+file-([a-z-]+)\s+("[^"]+"|\/\S.*)$/i;

export function parseFilesystemDenials(output: string): FilesystemDenial[] {
	const denials = new Map<string, FilesystemDenial>();
	for (const line of output.split("\n")) {
		const match = line.match(rawDenialPattern) ?? line.match(codexDenialPattern);
		if (!match) continue;
		const operation = match[1].toLowerCase();
		const rawPath = match[2].trim();
		const path = rawPath.startsWith('"') && rawPath.endsWith('"')
			? rawPath.slice(1, -1)
			: rawPath;
		const access = operation.includes("write") || operation.includes("create")
			? "write"
			: "read";
		denials.set(`${access}:${path}`, { access, path });
	}
	return [...denials.values()];
}
