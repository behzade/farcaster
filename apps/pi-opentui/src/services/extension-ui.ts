import type {
  ExtensionUIDialogOptions,
  ExtensionUIContext,
  Theme,
} from "@earendil-works/pi-coding-agent"
import {
  Deferred,
  Duration,
  Effect,
  Option,
  Ref,
  Runtime,
  Scope,
} from "effect"

export interface AppDialog {
  readonly id: number
  readonly kind: "select" | "input"
  readonly title: string
  readonly message: string | undefined
  readonly options: ReadonlyArray<string>
  readonly placeholder: string | undefined
}

export interface ExtensionUiHost {
  readonly setDialog: (
    dialog: AppDialog | undefined,
  ) => Effect.Effect<void>
  readonly notify: (
    message: string,
    isError: boolean,
  ) => Effect.Effect<void>
  readonly setStatus: (
    key: string,
    text: string | undefined,
  ) => Effect.Effect<void>
}

export interface ExtensionUiBridge {
  readonly context: ExtensionUIContext
  readonly notify: (message: string, isError: boolean) => void
  readonly resolveDialog: (
    id: number,
    value: string | undefined,
  ) => Effect.Effect<void>
  readonly cancelDialog: Effect.Effect<void>
}

const plainTheme = {
  fg: (_color: string, text: string) => text,
  bg: (_color: string, text: string) => text,
  bold: (text: string) => text,
  italic: (text: string) => text,
  underline: (text: string) => text,
  inverse: (text: string) => text,
  strikethrough: (text: string) => text,
  getFgAnsi: () => "",
  getBgAnsi: () => "",
  getColorMode: () => "truecolor",
  getThinkingBorderColor: () => (text: string) => text,
  getBashModeBorderColor: () => (text: string) => text,
} as unknown as Theme

export const makeExtensionUi = (
  host: ExtensionUiHost,
): Effect.Effect<ExtensionUiBridge, never, Scope.Scope> =>
  Effect.gen(function* () {
    const scope = yield* Effect.scope
    const runtime = yield* Effect.runtime<never>()
    const runFork = Runtime.runFork(runtime)
    const runPromise = Runtime.runPromise(runtime)
    const pendingDialog = yield* Ref.make<
      | {
          readonly id: number
          readonly result: Deferred.Deferred<string | undefined>
        }
      | undefined
    >(undefined)
    const nextDialogId = yield* Ref.make(1)

    const clearDialog = (id: number): Effect.Effect<void> =>
      Effect.gen(function* () {
        const pending = yield* Ref.get(pendingDialog)
        if (pending?.id === id) {
          yield* Ref.set(pendingDialog, undefined)
          yield* host.setDialog(undefined)
        }
      })

    const waitForAbort = (
      signal: AbortSignal | undefined,
    ): Effect.Effect<undefined> => {
      if (signal === undefined) return Effect.never
      if (signal.aborted) return Effect.succeed(undefined)

      return Effect.async<undefined>((resume) => {
        const onAbort = () => resume(Effect.succeed(undefined))
        signal.addEventListener("abort", onAbort, { once: true })
        return Effect.sync(() =>
          signal.removeEventListener("abort", onAbort),
        )
      })
    }

    const openDialogEffect = (
      dialog: Omit<AppDialog, "id">,
      options?: ExtensionUIDialogOptions,
    ): Effect.Effect<string | undefined> =>
      Effect.gen(function* () {
        const previous = yield* Ref.get(pendingDialog)
        if (previous !== undefined) {
          yield* Deferred.succeed(previous.result, undefined)
        }

        const id = yield* Ref.getAndUpdate(nextDialogId, (value) => value + 1)
        const result = yield* Deferred.make<string | undefined>()
        return yield* Effect.acquireUseRelease(
          Ref.set(pendingDialog, { id, result }).pipe(
            Effect.zipRight(host.setDialog({ ...dialog, id })),
          ),
          () => {
            const wait = Effect.race(
              Deferred.await(result),
              waitForAbort(options?.signal),
            )
            return options?.timeout === undefined
              ? wait
              : wait.pipe(
                  Effect.timeoutOption(
                    Duration.millis(options.timeout),
                  ),
                  Effect.map(Option.getOrUndefined),
                )
          },
          () => clearDialog(id),
        )
      })

    const openDialog = (
      dialog: Omit<AppDialog, "id">,
      options?: ExtensionUIDialogOptions,
    ): Promise<string | undefined> =>
      runPromise(openDialogEffect(dialog, options))

    let editorText = ""
    let editorFactory: Parameters<
      ExtensionUIContext["setEditorComponent"]
    >[0]
    let toolsExpanded = false

    const context: ExtensionUIContext = {
      select: (title, options, dialogOptions) =>
        openDialog(
          {
            kind: "select",
            title,
            message: undefined,
            options,
            placeholder: undefined,
          },
          dialogOptions,
        ),
      confirm: (title, message, dialogOptions) =>
        runPromise(
          openDialogEffect(
            {
              kind: "select",
              title,
              message,
              options: ["Yes", "No"],
              placeholder: undefined,
            },
            dialogOptions,
          ).pipe(Effect.map((value) => value === "Yes")),
        ),
      input: (title, placeholder, dialogOptions) =>
        openDialog(
          {
            kind: "input",
            title,
            message: undefined,
            options: [],
            placeholder,
          },
          dialogOptions,
        ),
      notify: (message, type) => {
        runFork(host.notify(message, type === "error"), { scope })
      },
      onTerminalInput: () => () => undefined,
      setStatus: (key, text) => {
        runFork(host.setStatus(key, text), { scope })
      },
      setWorkingMessage: (message) => {
        runFork(host.setStatus("working", message), { scope })
      },
      setWorkingVisible: () => undefined,
      setWorkingIndicator: () => undefined,
      setHiddenThinkingLabel: () => undefined,
      setWidget: () => undefined,
      setFooter: () => undefined,
      setHeader: () => undefined,
      setTitle: () => undefined,
      custom: <T>() =>
        runPromise(
          Effect.fail(
            new Error("Custom extension views are not supported yet"),
          ),
        ) as Promise<T>,
      pasteToEditor: (text) => {
        editorText += text
      },
      setEditorText: (text) => {
        editorText = text
      },
      getEditorText: () => editorText,
      editor: (title, prefill) =>
        openDialog({
          kind: "input",
          title,
          message: undefined,
          options: [],
          placeholder: prefill,
        }),
      addAutocompleteProvider: () => undefined,
      setEditorComponent: (factory) => {
        editorFactory = factory
      },
      getEditorComponent: () => editorFactory,
      theme: plainTheme,
      getAllThemes: () => [],
      getTheme: () => plainTheme,
      setTheme: () => ({ success: true }),
      getToolsExpanded: () => toolsExpanded,
      setToolsExpanded: (expanded) => {
        toolsExpanded = expanded
      },
    }

    const resolveDialog = (
      id: number,
      value: string | undefined,
    ): Effect.Effect<void> =>
      Effect.gen(function* () {
        const pending = yield* Ref.get(pendingDialog)
        if (pending?.id !== id) return
        yield* Deferred.succeed(pending.result, value)
      })

    const cancelDialog = Effect.gen(function* () {
      const pending = yield* Ref.get(pendingDialog)
      if (pending !== undefined) {
        yield* Deferred.succeed(pending.result, undefined)
      }
    })

    return {
      context,
      notify: (message, isError) => {
        runFork(host.notify(message, isError), { scope })
      },
      resolveDialog,
      cancelDialog,
    }
  })
