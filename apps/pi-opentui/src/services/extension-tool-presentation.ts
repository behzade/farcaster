import type {
  AgentToolResult,
  Theme,
  ToolDefinition,
} from "@earendil-works/pi-coding-agent"

export interface ExtensionToolPresentationInput {
  readonly toolCallId: string
  readonly toolName: string
  readonly args: unknown
  readonly result: unknown
  readonly pending: boolean
  readonly isError: boolean
}

export interface ExtensionToolPresentation {
  readonly label: string
  readonly call?: string
  readonly result?: string
}

export type PresentExtensionTool = (
  input: ExtensionToolPresentationInput,
) => ExtensionToolPresentation | undefined

interface RenderSlot {
  readonly state: Record<string, unknown>
  args: unknown
  callComponent?: { render(width: number): string[]; invalidate(): void }
  resultComponent?: { render(width: number): string[]; invalidate(): void }
}

const identity = (text: string): string => text

const plainTheme = {
  fg: (_color: string, text: string) => text,
  bg: (_color: string, text: string) => text,
  bold: identity,
  italic: identity,
  underline: identity,
  inverse: identity,
  strikethrough: identity,
  getFgAnsi: () => "",
  getBgAnsi: () => "",
  getColorMode: () => "truecolor",
  getThinkingBorderColor: () => identity,
  getBashModeBorderColor: () => identity,
} as unknown as Theme

const ansiPattern = new RegExp(
  "[\\u001B\\u009B][[\\]()#;?]*(?:(?:(?:[a-zA-Z\\d]*(?:;[-a-zA-Z\\d/#&.:=?%@~_]+)*)?\\u0007)|(?:(?:\\d{1,4}(?:[;:]\\d{0,4})*)?[\\dA-PR-TZcf-nq-uy=><~]))",
  "g",
)

const renderText = (
  component: { render(width: number): string[] } | undefined,
  width: number,
): string | undefined => {
  if (component === undefined) return undefined
  const text = component.render(width).join("\n").replace(ansiPattern, "").trim()
  return text.length > 0 ? text : undefined
}

export const createExtensionToolPresenter = (options: {
  readonly getDefinition: (name: string) => ToolDefinition | undefined
  readonly cwd: string
  readonly width?: number
}): PresentExtensionTool => {
  const slots = new Map<string, RenderSlot>()
  const width = options.width ?? 100

  return (input) => {
    const definition = options.getDefinition(input.toolName)
    if (definition === undefined) return undefined
    const slot = slots.get(input.toolCallId) ?? {
      state: {},
      args: input.args,
    }
    if (input.pending) slot.args = input.args
    slots.set(input.toolCallId, slot)
    const context = (lastComponent: RenderSlot["callComponent"]) => ({
      args: slot.args,
      toolCallId: input.toolCallId,
      invalidate: () => undefined,
      lastComponent,
      state: slot.state,
      cwd: options.cwd,
      executionStarted: true,
      argsComplete: true,
      isPartial: input.pending,
      expanded: false,
      showImages: false,
      isError: input.isError,
    })

    let call: string | undefined
    let result: string | undefined
    try {
      if (definition.renderCall !== undefined) {
        slot.callComponent = definition.renderCall(
          slot.args as never,
          plainTheme,
          context(slot.callComponent) as never,
        )
        call = renderText(slot.callComponent, width)
      }
      if (input.result !== undefined && definition.renderResult !== undefined) {
        slot.resultComponent = definition.renderResult(
          input.result as AgentToolResult<unknown>,
          { expanded: false, isPartial: input.pending },
          plainTheme,
          context(slot.resultComponent) as never,
        )
        result = renderText(slot.resultComponent, width)
      }
    } catch {
      // A display adapter must not break the tool or event stream.
    }

    if (!input.pending) slots.delete(input.toolCallId)
    return {
      label: definition.label,
      ...(call === undefined ? {} : { call }),
      ...(result === undefined ? {} : { result }),
    }
  }
}
