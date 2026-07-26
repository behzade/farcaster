import {
  AssistantMessageComponent,
  UserMessageComponent,
} from "@earendil-works/pi-coding-agent";

const PATCHED = Symbol.for("pi.dense-tools.chat-layout");
const EXTRA_LEFT_MARGIN = 1;
const USER_RULE = "#665c54"; // Gruvbox bg3
const ASSISTANT_RULE = "#3c3836"; // Gruvbox bg1

function foreground(color: string, text: string): string {
  const match = color.match(/^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i)!;
  return `\x1b[38;2;${parseInt(match[1], 16)};${parseInt(match[2], 16)};${parseInt(match[3], 16)}m${text}\x1b[0m`;
}

function rule(width: number, color: string): string {
  const contentWidth = Math.max(0, width - EXTRA_LEFT_MARGIN);
  return `${" ".repeat(EXTRA_LEFT_MARGIN)}${foreground(color, "─".repeat(contentWidth))}`;
}

function indent(line: string): string {
  // Keep shell-integration OSC markers at the start of the rendered line.
  return line.replace(/^((?:\x1b\][^\x07]*\x07)*)/, `$1${" ".repeat(EXTRA_LEFT_MARGIN)}`);
}

function patchRender(
  prototype: any,
  color: string,
  frame: "around" | "before",
): void {
  if (prototype[PATCHED]) return;
  const original = prototype.render;
  prototype.render = function renderWithSpacing(this: unknown, width: number): string[] {
    const lines = original.call(this, Math.max(1, width - EXTRA_LEFT_MARGIN));
    if (lines.length === 0) return lines;
    const body = lines.map(indent);
    return frame === "around"
      ? [rule(width, color), ...body, rule(width, color)]
      : [rule(width, color), ...body];
  };
  prototype[PATCHED] = true;
}

patchRender(UserMessageComponent.prototype, USER_RULE, "around");
patchRender(AssistantMessageComponent.prototype, ASSISTANT_RULE, "before");
