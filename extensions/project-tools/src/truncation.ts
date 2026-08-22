export const PROJECT_TOOL_MAX_LINES = 2_000;
export const PROJECT_TOOL_MAX_BYTES = 50 * 1024;

export interface ProjectToolTruncation {
  readonly totalLines: number;
  readonly totalBytes: number;
  readonly retainedLines: number;
  readonly retainedBytes: number;
  readonly maxLines: number;
  readonly maxBytes: number;
}

export interface BoundedProjectToolOutput {
  readonly text: string;
  readonly truncation?: ProjectToolTruncation;
}

function splitLines(content: string): string[] {
  if (content.length === 0) return [];
  const lines = content.split("\n");
  if (content.endsWith("\n")) lines.pop();
  return lines;
}

function utf8Prefix(content: string, maxBytes: number): string {
  if (Buffer.byteLength(content, "utf8") <= maxBytes) return content;
  const characters: string[] = [];
  let bytes = 0;
  for (const character of content) {
    const characterBytes = Buffer.byteLength(character, "utf8");
    if (bytes + characterBytes > maxBytes) break;
    characters.push(character);
    bytes += characterBytes;
  }
  return characters.join("");
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`;
  return `${(bytes / 1024).toFixed(1)}KB`;
}

export function truncateProjectToolOutput(output: string): BoundedProjectToolOutput {
  const totalLines = splitLines(output).length;
  const totalBytes = Buffer.byteLength(output, "utf8");
  if (totalLines <= PROJECT_TOOL_MAX_LINES && totalBytes <= PROJECT_TOOL_MAX_BYTES) {
    return { text: output };
  }

  const notice = `[Project tool output truncated: original output was ${totalLines} lines / ${formatSize(totalBytes)}; hard limit is ${PROJECT_TOOL_MAX_LINES} lines / ${formatSize(PROJECT_TOOL_MAX_BYTES)}. Refine the request for more specific output.]`;
  const separator = "\n\n";
  const retainedByteLimit = PROJECT_TOOL_MAX_BYTES - Buffer.byteLength(separator + notice, "utf8");
  const retainedLineLimit = PROJECT_TOOL_MAX_LINES - 2;
  const lineBounded = splitLines(output).slice(0, retainedLineLimit).join("\n");
  const retained = utf8Prefix(lineBounded, retainedByteLimit).replace(/\n+$/u, "");
  const text = `${retained}${separator}${notice}`;

  return {
    text,
    truncation: {
      totalLines,
      totalBytes,
      retainedLines: splitLines(retained).length,
      retainedBytes: Buffer.byteLength(retained, "utf8"),
      maxLines: PROJECT_TOOL_MAX_LINES,
      maxBytes: PROJECT_TOOL_MAX_BYTES,
    },
  };
}
