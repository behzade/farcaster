import * as pierre from "./diffs.bundle.mjs";
import * as standaloneTerminalText from "./terminal-text.ts";

export const THEME_NAME = "gruvbox-dark-hard";
// Gruvbox dark-hard, reduced by 15% so diff code stays distinct from the page.
export const DIFF_BACKGROUND = "#191b1c";
export const COLLAPSED_DIFF_ROWS = 30;

pierre.registerCustomTheme(THEME_NAME, async () => ({
  name: THEME_NAME,
  type: "dark",
  colors: {
    "editor.foreground": "#ebdbb2",
    "editor.background": DIFF_BACKGROUND,
  },
  settings: [
    { settings: { foreground: "#ebdbb2", background: DIFF_BACKGROUND } },
    { scope: ["comment", "punctuation.definition.comment"], settings: { foreground: "#928374", fontStyle: "italic" } },
    { scope: ["keyword", "storage.type", "storage.modifier"], settings: { foreground: "#fb4934" } },
    { scope: ["string", "string.quoted"], settings: { foreground: "#b8bb26" } },
    { scope: ["constant.numeric", "constant.language"], settings: { foreground: "#d3869b" } },
    { scope: ["entity.name.function", "support.function"], settings: { foreground: "#b8bb26" } },
    { scope: ["entity.name.type", "support.type", "entity.name.class"], settings: { foreground: "#fabd2f" } },
    { scope: ["variable", "meta.definition.variable.name"], settings: { foreground: "#83a598" } },
    { scope: ["constant", "entity.name.tag"], settings: { foreground: "#d3869b" } },
    { scope: ["punctuation", "keyword.operator"], settings: { foreground: "#fe8019" } },
  ],
}));

const highlighter = await pierre.getSharedHighlighter({
  themes: [THEME_NAME],
  langs: ["typescript", "tsx", "javascript", "jsx", "json", "css", "html", "markdown", "bash", "python", "rust", "go", "nix", "yaml", "toml"],
  preferredHighlighter: "shiki-js",
});

export interface DiffTheme {
  fg(color: "accent" | "borderMuted" | "dim" | "error" | "toolDiffContext", value: string): string;
}

interface TerminalText {
  sliceByColumn(value: string, start: number, width: number): string;
  truncateToWidth(value: string, width: number, ellipsis?: string): string;
  visibleWidth(value: string): number;
}

let terminalText: TerminalText = standaloneTerminalText;

export function useTerminalText(implementation: TerminalText): void {
  terminalText = implementation;
}

interface StyleState {
  color?: string;
  background?: string;
  inlineBackground?: string;
  italic?: boolean;
}

export interface PierreRows {
  deletions: any[];
  additions: any[];
}

interface ParsedFile {
  name?: string;
  prevName?: string;
}

export function ansi(style: StyleState): string {
  const codes: string[] = [];
  const match = style.color?.match(/^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i);
  if (match) codes.push(`38;2;${parseInt(match[1], 16)};${parseInt(match[2], 16)};${parseInt(match[3], 16)}`);
  const background = style.background?.match(/^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i);
  if (background) codes.push(`48;2;${parseInt(background[1], 16)};${parseInt(background[2], 16)};${parseInt(background[3], 16)}`);
  if (style.italic) codes.push("3");
  return codes.length ? `\x1b[${codes.join(";")}m` : "";
}

function styleFromNode(node: any, inherited: StyleState): StyleState {
  const next = { ...inherited };
  const css = typeof node.properties?.style === "string" ? node.properties.style : "";
  const color = css.match(/(?:^|;)color:\s*(#[0-9a-f]{6})/i)?.[1];
  if (color) next.color = color;
  if (/font-style:\s*italic/i.test(css)) next.italic = true;
  if (node.properties?.["data-diff-span"] !== undefined && next.inlineBackground) next.background = next.inlineBackground;
  return next;
}

function hastToAnsi(node: any, inherited: StyleState = {}): string {
  if (node.type === "text") return `${ansi(inherited)}${node.value}\x1b[0m`;
  const style = styleFromNode(node, inherited);
  return Array.isArray(node.children) ? node.children.map((child: any) => hastToAnsi(child, style)).join("") : "";
}

function lineIndexes(node: any): { unified: number; split: number } {
  const [unified = "0", split = unified] = String(node.properties?.["data-line-index"] ?? "0,0").split(",");
  return { unified: Number.parseInt(unified, 10), split: Number.parseInt(split, 10) };
}

function lineType(node: any): string {
  return String(node?.properties?.["data-line-type"] ?? "context");
}

function lineNumber(node: any): string {
  return node ? String(node.properties?.["data-line"] ?? "") : "";
}

function parsedFiles(patch: string): ParsedFile[] {
  const patches = pierre.parsePatchFiles(patch) ?? [];
  return patches.flatMap((parsed: any) => parsed.files ?? []);
}

function rowsForFile(file: ParsedFile): PierreRows {
  const rendered = pierre.renderDiffWithHighlighter(file, highlighter, {
    theme: THEME_NAME,
    useTokenTransformer: true,
    tokenizeMaxLineLength: 2000,
    lineDiffType: "word-alt",
    maxLineDiffLength: 2000,
  });
  return { deletions: rendered.code.deletionLines, additions: rendered.code.additionLines };
}

export function parsePatchRows(patch: string): PierreRows | undefined {
  const file = parsedFiles(patch)[0];
  return file ? rowsForFile(file) : undefined;
}

function fitCell(value: string, width: number, background?: string): string {
  const fitted = terminalText.truncateToWidth(value, width, "");
  const padding = " ".repeat(Math.max(0, width - terminalText.visibleWidth(fitted)));
  return background
    ? `${ansi({ background })}${fitted}${ansi({ background })}${padding}\x1b[0m`
    : `${fitted}${padding}`;
}

export function wrapByColumns(value: string, width: number): string[] {
  if (!value || width <= 0) return [""];
  const totalWidth = terminalText.visibleWidth(value);
  if (totalWidth <= width) return [value];

  const lines: string[] = [];
  let start = 0;
  while (start < totalWidth) {
    let chunk = terminalText.sliceByColumn(value, start, width);
    let chunkWidth = terminalText.visibleWidth(chunk);

    // A double-width character may begin in the final column. Move it to the
    // next line instead of cutting or dropping it.
    if (chunkWidth > width && width > 1) {
      chunk = terminalText.sliceByColumn(value, start, width - 1);
      chunkWidth = terminalText.visibleWidth(chunk);
    }
    if (chunkWidth === 0) {
      chunk = terminalText.sliceByColumn(value, start, 1);
      chunkWidth = terminalText.visibleWidth(chunk);
    }

    lines.push(chunk);
    start += chunkWidth;
  }
  return lines;
}

export function wrapCell(prefix: string, content: string, width: number, background?: string): string[] {
  const prefixWidth = terminalText.visibleWidth(prefix);
  if (width <= prefixWidth) return wrapByColumns(`${prefix}${content}`, width).map((line) => fitCell(line, width, background));

  const contentWidth = width - prefixWidth;
  const continuationPrefix = " ".repeat(prefixWidth);
  return wrapByColumns(content, contentWidth).map((line, index) => {
    return fitCell(`${index === 0 ? prefix : continuationPrefix}${line}`, width, background);
  });
}

function diffColors(type: string): { marker: string; foreground?: string; background?: string; inlineBackground?: string } {
  if (type.includes("deletion")) {
    return { marker: "-", foreground: "#fb4934", background: "#412724", inlineBackground: "#682e27" };
  }
  if (type.includes("addition")) {
    return { marker: "+", foreground: "#b8bb26", background: "#363922", inlineBackground: "#525523" };
  }
  return { marker: " ", background: DIFF_BACKGROUND };
}

function capRows<T>(rows: T[], expanded: boolean): { visible: T[]; hidden: number } {
  if (expanded || rows.length <= COLLAPSED_DIFF_ROWS) return { visible: rows, hidden: 0 };
  return {
    visible: rows.slice(0, COLLAPSED_DIFF_ROWS),
    hidden: rows.length - COLLAPSED_DIFF_ROWS,
  };
}

function addHiddenRows(lines: string[], hidden: number, theme: DiffTheme): string[] {
  if (hidden === 0) return lines;
  return [...lines, theme.fg("dim", `… ${hidden} more diff rows (Ctrl+O to expand)`)];
}

function frameDiff(lines: string[], width: number, theme: DiffTheme): string[] {
  const rule = theme.fg("borderMuted", "─".repeat(Math.max(0, width)));
  return [rule, ...lines, rule];
}

export function renderUnified(rows: PierreRows, width: number, expanded: boolean, theme: DiffTheme): string[] {
  const changed = [...rows.deletions, ...rows.additions].filter((row) => lineType(row) !== "context");
  const contextByIndex = new Map<number, any>();
  for (const row of rows.deletions) if (lineType(row) === "context") contextByIndex.set(lineIndexes(row).unified, row);
  for (const row of rows.additions) if (lineType(row) === "context") contextByIndex.set(lineIndexes(row).unified, row);
  const ordered = [...changed, ...contextByIndex.values()].sort((a, b) => lineIndexes(a).unified - lineIndexes(b).unified);
  const numberWidth = Math.max(1, ...ordered.map((row) => lineNumber(row).length));
  const capped = capRows(ordered, expanded);
  const body = capped.visible.flatMap((row) => {
    const colors = diffColors(lineType(row));
    const prefix = colors.foreground
      ? `${ansi({ color: colors.foreground, background: colors.background })}${colors.marker}${lineNumber(row).padStart(numberWidth)} \x1b[0m`
      : theme.fg("toolDiffContext", ` ${lineNumber(row).padStart(numberWidth)} `);
    const content = hastToAnsi(row, { background: colors.background, inlineBackground: colors.inlineBackground });
    return wrapCell(prefix, content, width, colors.background);
  });
  return frameDiff(addHiddenRows(body, capped.hidden, theme), width, theme);
}

export function renderSplit(rows: PierreRows, width: number, expanded: boolean, theme: DiffTheme): string[] {
  const oldBySplit = new Map(rows.deletions.map((row) => [lineIndexes(row).split, row]));
  const newBySplit = new Map(rows.additions.map((row) => [lineIndexes(row).split, row]));
  const indexes = [...new Set([...oldBySplit.keys(), ...newBySplit.keys()])].sort((a, b) => a - b);
  const capped = capRows(indexes, expanded);
  const numberWidth = Math.max(1, ...[...rows.deletions, ...rows.additions].map((row) => lineNumber(row).length));
  const separator = theme.fg("borderMuted", "│");
  const cellWidth = Math.floor((width - 1) / 2);

  const renderCell = (row: any, side: "old" | "new"): { lines: string[]; background: string } => {
    if (!row) return { lines: [fitCell("", cellWidth, DIFF_BACKGROUND)], background: DIFF_BACKGROUND };
    const type = lineType(row);
    const changed = side === "old" ? type.includes("deletion") : type.includes("addition");
    const colors = changed ? diffColors(type) : diffColors("context");
    const prefix = colors.foreground
      ? `${ansi({ color: colors.foreground, background: colors.background })}${colors.marker}${lineNumber(row).padStart(numberWidth)} \x1b[0m`
      : theme.fg("toolDiffContext", ` ${lineNumber(row).padStart(numberWidth)} `);
    const content = hastToAnsi(row, { background: colors.background, inlineBackground: colors.inlineBackground });
    return {
      lines: wrapCell(prefix, content, cellWidth, colors.background),
      background: colors.background ?? DIFF_BACKGROUND,
    };
  };

  const body = capped.visible.flatMap((index) => {
    const oldCell = renderCell(oldBySplit.get(index), "old");
    const newCell = renderCell(newBySplit.get(index), "new");
    const height = Math.max(oldCell.lines.length, newCell.lines.length);
    return Array.from({ length: height }, (_, lineIndex) => {
      const oldLine = oldCell.lines[lineIndex] ?? fitCell("", cellWidth, oldCell.background);
      const newLine = newCell.lines[lineIndex] ?? fitCell("", cellWidth, newCell.background);
      return `${oldLine}${separator}${newLine}`;
    });
  });
  return frameDiff(addHiddenRows(body, capped.hidden, theme), width, theme);
}

function displayName(file: ParsedFile, roots: string[]): string {
  const names = [file.prevName, file.name].filter((name): name is string => Boolean(name));
  const cleaned = names.map((name) => {
    const normalized = name.replaceAll("\\", "/");
    const root = roots.find((candidate) => normalized === candidate || normalized.startsWith(`${candidate}/`));
    return root ? normalized.slice(root.length).replace(/^\/+/, "") : normalized.replace(/^[ab]\//, "");
  });
  if (cleaned.length === 2 && cleaned[0] !== cleaned[1]) return `${cleaned[0] || names[0]} → ${cleaned[1] || names[1]}`;
  return cleaned.at(-1) || names.at(-1) || "(changed file)";
}

export function renderPatch(patch: string, width: number, theme: DiffTheme, roots: string[] = []): string[] {
  const files = parsedFiles(patch);
  return files.flatMap((file, index) => {
    const rows = rowsForFile(file);
    const title = theme.fg("accent", displayName(file, roots));
    const rendered = width >= 120
      ? renderSplit(rows, width, true, theme)
      : renderUnified(rows, width, true, theme);
    const body = rows.deletions.length || rows.additions.length
      ? rendered
      : frameDiff([theme.fg("dim", "Binary or metadata-only change")], width, theme);
    return index === 0 ? [title, ...body] : ["", title, ...body];
  });
}
