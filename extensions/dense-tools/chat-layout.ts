import {
  AssistantMessageComponent,
  CustomEditor,
  type ExtensionContext,
  ToolExecutionComponent,
  UserMessageComponent,
} from "@earendil-works/pi-coding-agent";

const PATCHED = Symbol.for("pi.dense-tools.chat-layout");
const MARGIN_EDITOR = Symbol.for("pi.dense-tools.margin-editor");
const EXTRA_LEFT_MARGIN = 1;
const COMPOSER_PADDING = 2;

function indent(line: string): string {
  // Keep shell-integration OSC markers at the start of the rendered line.
  return line.replace(/^((?:\x1b\][^\x07]*\x07)*)/, `$1${" ".repeat(EXTRA_LEFT_MARGIN)}`);
}

function patchRender(prototype: any): void {
  if (prototype[PATCHED]) return;
  const original = prototype.render;
  prototype.render = function renderWithSpacing(this: unknown, width: number): string[] {
    const lines = original.call(this, Math.max(1, width - EXTRA_LEFT_MARGIN));
    return lines.map(indent);
  };
  prototype[PATCHED] = true;
}

patchRender(UserMessageComponent.prototype);
patchRender(AssistantMessageComponent.prototype);
patchRender(ToolExecutionComponent.prototype);

export function setComposerMargin(ctx: ExtensionContext): void {
  const previous = ctx.ui.getEditorComponent();
  if ((previous as any)?.[MARGIN_EDITOR]) return;
  const factory = (tui: any, theme: any, keybindings: any) => {
    const editor = previous
      ? previous(tui, theme, keybindings)
      : new CustomEditor(tui, theme, keybindings);
    editor.setPaddingX?.(COMPOSER_PADDING);
    return editor;
  };
  (factory as any)[MARGIN_EDITOR] = true;
  ctx.ui.setEditorComponent(factory);
}
