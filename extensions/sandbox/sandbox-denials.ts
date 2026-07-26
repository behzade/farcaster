export interface FilesystemDenial {
	access: "read" | "write";
	path: string;
}

const denialPattern = /\bdeny(?:\(\d+\))?\s+file-([a-z-]+)\s+("[^"]+"|\/\S.*)$/i;

export function parseFilesystemDenials(output: string): FilesystemDenial[] {
	const denials = new Map<string, FilesystemDenial>();
	for (const line of output.split("\n")) {
		const match = line.match(denialPattern);
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
