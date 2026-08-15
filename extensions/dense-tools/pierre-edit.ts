import type { EditToolDetails, ExtensionAPI, Theme } from "@earendil-works/pi-coding-agent";
import { createEditTool } from "@earendil-works/pi-coding-agent";
import {
  sliceByColumn,
  Text,
  truncateToWidth,
  visibleWidth,
} from "@earendil-works/pi-tui";
import {
  parsePatchRows,
  renderSplit,
  renderUnified,
  THEME_NAME,
  type PierreRows,
  useTerminalText,
} from "./pierre-renderer.ts";
import { selectThemeWhenAvailable } from "./theme-selection.ts";

useTerminalText({ sliceByColumn, truncateToWidth, visibleWidth });

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
    selectThemeWhenAvailable(ctx.ui, THEME_NAME);
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
