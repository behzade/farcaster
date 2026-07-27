import type { EditToolDetails, ExtensionAPI, Theme } from "@earendil-works/pi-coding-agent";
import { createEditTool } from "@earendil-works/pi-coding-agent";
import { sliceByColumn, Text, truncateToWidth, visibleWidth } from "@earendil-works/pi-tui";
import * as pierre from "./diffs.bundle.mjs";

const THEME_NAME = "gruvbox-dark-hard";
// Gruvbox dark-hard, reduced by 15% so diff code stays distinct from the page.
const DIFF_BACKGROUND = "#191b1c";
const COLLAPSED_DIFF_ROWS = 30;

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

interface StyleState {
  color?: string;
  background?: string;
  inlineBackground?: string;
  italic?: boolean;
}

function ansi(style: StyleState): string {
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

interface PierreRows {
  deletions: any[];
  additions: any[];
}

function parsePatchRows(patch: string): PierreRows | undefined {
  const file = pierre.parsePatchFiles(patch)?.[0]?.files?.[0];
  if (!file) return undefined;
  const rendered = pierre.renderDiffWithHighlighter(file, highlighter, {
    theme: THEME_NAME,
    useTokenTransformer: true,
    tokenizeMaxLineLength: 2000,
    lineDiffType: "word-alt",
    maxLineDiffLength: 2000,
  });
  return { deletions: rendered.code.deletionLines, additions: rendered.code.additionLines };
}

function fitCell(value: string, width: number, background?: string): string {
  const fitted = truncateToWidth(value, width, "");
  const padding = " ".repeat(Math.max(0, width - visibleWidth(fitted)));
  return background
    ? `${ansi({ background })}${fitted}${ansi({ background })}${padding}\x1b[0m`
    : `${fitted}${padding}`;
}

export function wrapByColumns(value: string, width: number): string[] {
  if (!value || width <= 0) return [""];
  const totalWidth = visibleWidth(value);
  if (totalWidth <= width) return [value];

  const lines: string[] = [];
  let start = 0;
  while (start < totalWidth) {
    let chunk = sliceByColumn(value, start, width);
    let chunkWidth = visibleWidth(chunk);

    // A double-width character may begin in the final column. Move it to the
    // next line instead of cutting or dropping it.
    if (chunkWidth > width && width > 1) {
      chunk = sliceByColumn(value, start, width - 1);
      chunkWidth = visibleWidth(chunk);
    }
    if (chunkWidth === 0) {
      chunk = sliceByColumn(value, start, 1);
      chunkWidth = visibleWidth(chunk);
    }

    lines.push(chunk);
    start += chunkWidth;
  }
  return lines;
}

export function wrapCell(prefix: string, content: string, width: number, background?: string): string[] {
  const prefixWidth = visibleWidth(prefix);
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

function addHiddenRows(lines: string[], hidden: number, theme: Theme): string[] {
  if (hidden === 0) return lines;
  return [...lines, theme.fg("dim", `… ${hidden} more diff rows (Ctrl+O to expand)`)];
}

function frameDiff(lines: string[], width: number, theme: Theme): string[] {
  const rule = theme.fg("borderMuted", "─".repeat(Math.max(0, width)));
  return [rule, ...lines, rule];
}

export function renderUnified(rows: PierreRows, width: number, expanded: boolean, theme: Theme): string[] {
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

export function renderSplit(rows: PierreRows, width: number, expanded: boolean, theme: Theme): string[] {
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

class PierreDiffComponent {
  private rows?: PierreRows;
  private error?: string;
  private cachedWidth?: number;
  private cachedLines?: string[];

  constructor(private patch: string, private expanded: boolean, private theme: Theme) {
    this.rebuild();
  }

  set(patch: string, expanded: boolean, theme: Theme): void {
    const patchChanged = patch !== this.patch;
    const renderChanged = expanded !== this.expanded || theme !== this.theme;
    this.patch = patch;
    this.expanded = expanded;
    this.theme = theme;
    if (patchChanged) this.rebuild();
    else if (renderChanged) this.clearRenderCache();
  }

  private clearRenderCache(): void {
    this.cachedWidth = undefined;
    this.cachedLines = undefined;
  }

  private rebuild(): void {
    this.rows = parsePatchRows(this.patch);
    this.error = this.rows ? undefined : "Pierre could not parse the edit patch";
    this.clearRenderCache();
  }

  render(width: number): string[] {
    if (this.cachedLines && this.cachedWidth === width) return this.cachedLines;
    const lines = !this.rows
      ? [this.theme.fg("error", this.error ?? "Could not render diff")]
      : width >= 120
        ? renderSplit(this.rows, width, this.expanded, this.theme)
        : renderUnified(this.rows, width, this.expanded, this.theme);
    this.cachedWidth = width;
    this.cachedLines = lines;
    return lines;
  }

  invalidate(): void {
    this.clearRenderCache();
  }
}

export default function (pi: ExtensionAPI) {
  const edit = createEditTool(process.cwd());

  pi.on("session_start", (_event, ctx) => {
    const selected = ctx.ui.setTheme(THEME_NAME);
    if (!selected.success) ctx.ui.notify(selected.error ?? `Could not select ${THEME_NAME}`, "error");
  });

  pi.registerTool({
    ...edit,
    renderShell: "self",
    renderCall(args, theme, context) {
      const text = context.lastComponent instanceof Text ? context.lastComponent : new Text("", 0, 0);
      text.setText(`${theme.fg("toolTitle", theme.bold("edit"))} ${theme.fg("accent", args.path)}`);
      return text;
    },
    renderResult(result, options, theme, context) {
      if (context.isError) {
        const message = result.content.filter((item) => item.type === "text").map((item: any) => item.text).join("\n");
        return new Text(theme.fg("error", message), 0, 0);
      }
      const details = result.details as EditToolDetails | undefined;
      if (!details?.patch) return new Text(theme.fg("success", "edited"), 0, 0);
      const component = context.lastComponent instanceof PierreDiffComponent
        ? context.lastComponent
        : new PierreDiffComponent(details.patch, options.expanded, theme);
      component.set(details.patch, options.expanded, theme);
      return component;
    },
  });
}
