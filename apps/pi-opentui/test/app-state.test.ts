import {
  type AgentSessionEvent,
  type ExtensionUIContext,
} from "@earendil-works/pi-coding-agent"
import { expect, test } from "bun:test"
import { Effect, Layer, Stream } from "effect"
import {
  AppState,
  AppStateLive,
} from "../src/services/app-state.ts"
import { Keybindings } from "../src/services/keybindings.ts"
import {
  PiSession,
  PiSessionError,
  type PiModelState,
} from "../src/services/pi-session.ts"
import { headerViewModel } from "../src/ui/app-view-model.ts"
import { activityError } from "../src/services/app-activity.ts"

test("folds session events and commands into app state", () => {
  let emit: ((event: AgentSessionEvent) => void) | undefined
  let extensionUi: ExtensionUIContext | undefined
  let prompt = ""
  let promptCalls = 0
  let failQueuedPrompt = false
  const promptRequests: Array<{
    text: string
    delivery: "steer" | "followUp" | undefined
  }> = []
  const directQueueRequests: Array<{
    text: string
    delivery: "steer" | "followUp"
  }> = []
  let abortCompactionCalls = 0
  let finishPrompt: (() => void) | undefined
  let failPrompt: (() => void) | undefined
  let compactInstructions: string | undefined
  let holdCompaction = false
  let finishCompaction: (() => void) | undefined
  let failCompaction: (() => void) | undefined
  let newSessionCalls = 0
  let resumedPath: string | undefined
  let selectedModel: string | undefined
  let selectedThinking: string | undefined
  let savedLogin: string | undefined
  let reloadCalls = 0
  let keybindingReloads = 0
  let queuedForClear = {
    steering: [] as Array<string>,
    followUp: [] as Array<string>,
  }
  let sessionStatsReads = 0
  let authoritativeTokens = 30
  let authoritativeCost = 0.01
  let authoritativeContextPercent = 10
  let modelState: PiModelState = {
    selected: {
      provider: "openai",
      id: "gpt-5",
      name: "GPT-5",
      reasoning: true,
    },
    thinkingLevel: "medium",
    thinkingLevels: ["off", "low", "medium", "high"],
  }

  const events = Stream.asyncPush<AgentSessionEvent>((push) =>
    Effect.sync(() => {
      emit = (event) => {
        push.single(event)
      }
    }),
  )

  const pi = Layer.succeed(PiSession, {
    cwd: "/work",
    hideThinkingBlock: false,
    activeTools: ["sandbox"],
    extensionPaths: ["/agent/extensions/sandbox"],
    extensionErrors: [],
    events,
    commands: Effect.succeed([
      {
        name: "review",
        description: "Review changes",
        source: "extension" as const,
        sourceInfo: {
          path: "/agent/extensions/review.ts",
          source: "review",
          scope: "user" as const,
          origin: "top-level" as const,
        },
      },
      {
        name: "session",
        description: "Extension conflict",
        source: "extension" as const,
        sourceInfo: {
          path: "/agent/extensions/session.ts",
          source: "session",
          scope: "user" as const,
          origin: "top-level" as const,
        },
      },
    ]),
    sessionStats: Effect.sync(() => {
      sessionStatsReads += 1
      return {
        sessionFile: undefined,
        sessionId: "session-1",
        userMessages: 1,
        assistantMessages: 1,
        toolCalls: 0,
        toolResults: 0,
        totalMessages: 2,
        tokens: {
          input: 10,
          output: authoritativeTokens - 10,
          cacheRead: 0,
          cacheWrite: 0,
          total: authoritativeTokens,
        },
        cost: authoritativeCost,
        contextUsage: {
          tokens: authoritativeContextPercent * 1_000,
          contextWindow: 100_000,
          percent: authoritativeContextPercent,
        },
      }
    }),
    modelState: Effect.sync(() => modelState),
    models: Effect.succeed([
      {
        provider: "anthropic",
        id: "claude-sonnet",
        name: "Claude Sonnet",
        reasoning: true,
      },
      {
        provider: "openai",
        id: "gpt-5",
        name: "GPT-5",
        reasoning: true,
      },
      {
        provider: "opencode-go",
        id: "glm-5.2",
        name: "GLM 5.2",
        reasoning: true,
      },
    ]),
    authProviders: Effect.succeed([
      {
        id: "opencode-go",
        name: "OpenCode Go",
        type: "api_key" as const,
        methodName: "OpenCode Go API key",
        loginLabel: undefined,
        interactive: true,
        configured: false,
        source: undefined,
      },
    ]),
    sessions: Effect.succeed([
      {
        path: "/sessions/old.jsonl",
        id: "old-session",
        cwd: "/work",
        created: new Date("2026-01-01T00:00:00Z"),
        modified: new Date("2026-01-02T00:00:00Z"),
        messageCount: 2,
        firstMessage: "old question",
        allMessagesText: "old question old answer",
      },
    ]),
    messages: Effect.succeed([]),
    presentExtensionTool: () => undefined,
    prompt: (text, delivery) => {
      promptRequests.push({ text, delivery })
      if (text.startsWith("/after-reload") && delivery === undefined) {
        return Effect.void
      }
      if (delivery !== undefined) {
        return failQueuedPrompt
          ? Effect.fail(
              new PiSessionError({
                operation: "prompt",
                cause: new Error("Could not queue prompt"),
              }),
            )
          : Effect.void
      }
      return Effect.async<void, PiSessionError>((resume) => {
        prompt = text
        promptCalls += 1
        finishPrompt = () => resume(Effect.void)
        failPrompt = () =>
          resume(
            Effect.fail(
              new PiSessionError({
                operation: "prompt",
                cause: new Error("Request was aborted"),
              }),
            ),
          )
      })
    },
    queuePrompt: (text, delivery) =>
      Effect.sync(() => {
        directQueueRequests.push({ text, delivery })
      }),
    clearQueue: Effect.sync(() => {
      const queued = queuedForClear
      queuedForClear = { steering: [], followUp: [] }
      return queued
    }),
    compact: (instructions) =>
      Effect.sync(() => {
        compactInstructions = instructions
      }).pipe(
        Effect.zipRight(
          holdCompaction
            ? Effect.async<void, PiSessionError>((resume) => {
                finishCompaction = () => resume(Effect.void)
                failCompaction = () =>
                  resume(
                    Effect.fail(
                      new PiSessionError({
                        operation: "compact",
                        cause: new Error("Compaction cancelled"),
                      }),
                    ),
                  )
              })
            : Effect.void,
        ),
      ),
    reload: Effect.sync(() => {
      reloadCalls += 1
      return {
        hideThinkingBlock: true,
        activeTools: ["read", "sandbox"],
        extensionPaths: ["/agent/extensions/reloaded"],
        extensionErrors: [],
        commands: [
          {
            name: "after-reload",
            description: "Loaded after reload",
            source: "extension" as const,
            sourceInfo: {
              path: "/agent/extensions/reloaded.ts",
              source: "after-reload",
              scope: "user" as const,
              origin: "top-level" as const,
            },
          },
        ],
        modelState,
      }
    }),
    newSession: Effect.sync(() => {
      newSessionCalls += 1
      return []
    }),
    resume: (path) =>
      Effect.sync(() => {
        resumedPath = path
        return [
          { role: "user", content: "old question" },
          {
            role: "assistant",
            content: [{ type: "text", text: "old answer" }],
          },
        ]
      }),
    selectModel: (provider, id) =>
      Effect.sync(() => {
        selectedModel = `${provider}/${id}`
        modelState = {
          ...modelState,
          selected: {
            provider,
            id,
            name: id,
            reasoning: true,
          },
        }
        return modelState
      }),
    selectThinking: (level) =>
      Effect.sync(() => {
        selectedThinking = level
        modelState = {
          ...modelState,
          thinkingLevel: level,
        }
        return modelState
      }),
    login: (provider, type, interaction) =>
      Effect.tryPromise({
        try: () => {
          interaction.notify({
            type: "auth_url",
            url: "https://login.example.test/device",
            instructions: "Open the login page",
          })
          return interaction.prompt({
            type: "secret",
            message: "Enter API key",
            placeholder: "key",
          })
        },
        catch: (cause) =>
          new PiSessionError({ operation: "login", cause }),
      }).pipe(
        Effect.tap((key) =>
          Effect.sync(() => {
            savedLogin = `${provider}/${type}/${key}`
          }),
        ),
        Effect.asVoid,
      ),
    abort: Effect.sync(() => {
      failPrompt?.()
    }),
    abortCompaction: Effect.sync(() => {
      abortCompactionCalls += 1
      emit?.({
        type: "compaction_end",
        reason: "manual",
        result: undefined,
        aborted: true,
        willRetry: false,
      })
      failCompaction?.()
    }),
    bindExtensions: (ui) =>
      Effect.sync(() => {
        extensionUi = ui
      }),
  })
  const keybindings = Layer.succeed(Keybindings, {
    matches: () => false,
    keys: () => [],
    reload: () => {
      keybindingReloads += 1
    },
  })
  const appLayer = AppStateLive.pipe(
    Layer.provide(Layer.merge(pi, keybindings)),
  )

  const program = Effect.scoped(
    Effect.gen(function* () {
      const app = yield* AppState

      while (emit === undefined) {
        yield* Effect.yieldNow()
      }
      emit({ type: "agent_settled" })

      let snapshot = yield* app.get
      while (snapshot.eventCount === 0) {
        yield* Effect.yieldNow()
        snapshot = yield* app.get
      }

      expect(snapshot.lastEvent).toBe("agent_settled")
      expect(snapshot.sessionStats.tokens.total).toBe(30)
      expect(sessionStatsReads).toBeGreaterThanOrEqual(2)

      emit({
        type: "agent_start",
      })
      emit({
        type: "message_update",
        message: {
          role: "assistant",
          content: [{ type: "text", text: "working" }],
          usage: {
            input: 5,
            output: 2,
            cacheRead: 0,
            cacheWrite: 0,
            totalTokens: 7,
            cost: { total: 0.005 },
          },
        },
        assistantMessageEvent: {
          type: "text_delta",
          delta: "working",
        },
      } as AgentSessionEvent)
      while ((yield* app.get).lastEvent !== "message_update") {
        yield* Effect.yieldNow()
      }
      snapshot = yield* app.get
      expect(snapshot.liveUsage.current.total).toBe(7)
      expect(headerViewModel(snapshot).usage).toContain("37 tokens")
      expect(headerViewModel(snapshot).usage).toContain("$0.0150")

      emit({ type: "thinking_level_changed", level: "high" })
      while ((yield* app.get).lastEvent !== "thinking_level_changed") {
        yield* Effect.yieldNow()
      }
      expect((yield* app.get).thinkingLevel).toBe("high")

      authoritativeContextPercent = 42
      const readsBeforeMessageEnd = sessionStatsReads
      emit({
        type: "message_end",
        message: {
          role: "assistant",
          content: [{ type: "text", text: "done" }],
          usage: {
            input: 7,
            output: 3,
            cacheRead: 0,
            cacheWrite: 0,
            totalTokens: 10,
            cost: { total: 0.006 },
          },
        },
      } as AgentSessionEvent)
      while ((yield* app.get).lastEvent !== "message_end") {
        yield* Effect.yieldNow()
      }
      snapshot = yield* app.get
      expect(snapshot.liveUsage.completed.total).toBe(10)
      expect(headerViewModel(snapshot).usage).toContain("40 tokens")
      expect(headerViewModel(snapshot).usage).toContain("ctx 42%")
      expect(sessionStatsReads).toBeGreaterThan(readsBeforeMessageEnd)

      authoritativeTokens = 40
      authoritativeCost = 0.016
      emit({ type: "agent_settled" })
      while (
        (yield* app.get).lastEvent !== "agent_settled" ||
        (yield* app.get).liveUsage.completed.total !== 0
      ) {
        yield* Effect.yieldNow()
      }
      expect(headerViewModel(yield* app.get).usage).toContain("40 tokens")
      expect(snapshot.commands.map((command) => command.name)).toContain(
        "review",
      )
      expect(
        snapshot.commands.filter((command) => command.name === "session"),
      ).toEqual([
        {
          name: "session",
          description: "Show session info and stats",
          source: "builtin",
        },
      ])
      extensionUi?.notify("sandbox ready")
      while (
        !(yield* app.get).transcript.rows.some(
          (row) => row.content === "sandbox ready",
        )
      ) {
        yield* Effect.yieldNow()
      }

      const selection = extensionUi?.select("Allow write?", [
        "Allow once",
        "Deny",
      ])
      let dialog = (yield* app.get).dialog
      while (dialog === undefined) {
        yield* Effect.yieldNow()
        dialog = (yield* app.get).dialog
      }
      yield* app.dispatch({
        _tag: "ResolveDialog",
        id: dialog.id,
        value: "Allow once",
      })
      expect(
        yield* Effect.promise(() =>
          selection ?? Promise.resolve(undefined),
        ),
      ).toBe("Allow once")

      yield* app.dispatch({
        _tag: "Prompt",
        text: "  test prompt  ",
        delivery: "steer",
      })
      yield* app.dispatch({
        _tag: "Prompt",
        text: "second prompt",
        delivery: "steer",
      })
      while (prompt.length === 0) {
        yield* Effect.yieldNow()
      }
      expect(prompt).toBe("test prompt")
      expect((yield* app.get).activity._tag).toBe("Turn")
      expect(promptCalls).toBe(1)
      while (
        !promptRequests.some(
          (request) =>
            request.text === "second prompt" &&
            request.delivery === "steer",
        )
      ) {
        yield* Effect.yieldNow()
      }
      expect(promptRequests).toContainEqual({
        text: "second prompt",
        delivery: "steer",
      })

      yield* app.dispatch({
        _tag: "Prompt",
        text: "after this turn",
        delivery: "followUp",
      })
      while (
        !promptRequests.some(
          (request) =>
            request.text === "after this turn" &&
            request.delivery === "followUp",
        )
      ) {
        yield* Effect.yieldNow()
      }
      expect(promptRequests).toContainEqual({
        text: "after this turn",
        delivery: "followUp",
      })

      emit?.({
        type: "queue_update",
        steering: ["second prompt"],
        followUp: ["after this turn"],
      })
      while ((yield* app.get).lastEvent !== "queue_update") {
        yield* Effect.yieldNow()
      }
      expect((yield* app.get).promptQueue).toEqual({
        steering: ["second prompt"],
        followUp: ["after this turn"],
      })

      failQueuedPrompt = true
      yield* app.dispatch({
        _tag: "Prompt",
        text: "queue failure",
        delivery: "steer",
      })
      while ((yield* app.get).draftRestore?.text !== "queue failure") {
        yield* Effect.yieldNow()
      }
      expect((yield* app.get).activity._tag).toBe("Turn")
      expect(
        (yield* app.get).transcript.rows.some((row) =>
          row.content.includes("Could not queue prompt"),
        ),
      ).toBe(true)
      const failedRestore = (yield* app.get).draftRestore
      expect(failedRestore?.text).toBe("queue failure")
      expect((yield* app.get).promptQueue).toEqual({
        steering: ["second prompt"],
        followUp: ["after this turn"],
      })
      yield* app.dispatch({
        _tag: "AcknowledgeDraftRestore",
        id: failedRestore?.id ?? -1,
      })
      while ((yield* app.get).draftRestore !== undefined) {
        yield* Effect.yieldNow()
      }
      failQueuedPrompt = false

      emit?.({
        type: "message_start",
        message: { role: "user", content: "second prompt" },
      } as AgentSessionEvent)
      while ((yield* app.get).lastEvent !== "message_start") {
        yield* Effect.yieldNow()
      }
      expect(
        (yield* app.get).transcript.rows.some(
          (row) => row.kind === "user" && row.content === "second prompt",
        ),
      ).toBe(true)

      queuedForClear = {
        steering: ["second prompt"],
        followUp: ["after this turn"],
      }
      yield* app.dispatch({ _tag: "Dequeue" })
      while ((yield* app.get).draftRestore === undefined) {
        yield* Effect.yieldNow()
      }
      const restore = (yield* app.get).draftRestore
      expect(restore?.text).toBe("second prompt\n\nafter this turn")
      expect((yield* app.get).promptQueue).toEqual({
        steering: [],
        followUp: [],
      })
      yield* app.dispatch({
        _tag: "AcknowledgeDraftRestore",
        id: restore?.id ?? -1,
      })
      while ((yield* app.get).draftRestore !== undefined) {
        yield* Effect.yieldNow()
      }
      finishPrompt?.()
      while ((yield* app.get).activity._tag !== "Idle") {
        yield* Effect.yieldNow()
      }
      expect((yield* app.get).activity._tag).toBe("Idle")

      prompt = ""
      yield* app.dispatch({
        _tag: "Prompt",
        text: "stop me",
        delivery: "steer",
      })
      while (prompt !== "stop me") {
        yield* Effect.yieldNow()
      }
      queuedForClear = {
        steering: ["put this back"],
        followUp: ["and this"],
      }
      yield* app.dispatch({ _tag: "Abort" })
      while ((yield* app.get).activity._tag !== "Idle") {
        yield* Effect.yieldNow()
      }
      expect(activityError((yield* app.get).activity)).toBeUndefined()
      expect((yield* app.get).draftRestore?.text).toBe(
        "put this back\n\nand this",
      )
      expect((yield* app.get).promptQueue).toEqual({
        steering: [],
        followUp: [],
      })
      expect(
        (yield* app.get).transcript.rows.some((row) =>
          row.content.includes("Request was aborted"),
        ),
      ).toBe(false)

      const callsBeforeCommands = promptCalls
      yield* app.dispatch({
        _tag: "RunCommand",
        name: "session",
        arguments: "",
        delivery: "steer",
      })
      while (
        !(yield* app.get).transcript.rows.some((row) =>
          row.content.includes("Session session-1"),
        )
      ) {
        yield* Effect.yieldNow()
      }
      expect(promptCalls).toBe(callsBeforeCommands)

      yield* app.dispatch({
        _tag: "RunCommand",
        name: "reload",
        arguments: "",
        delivery: "steer",
      })
      while (reloadCalls === 0 || keybindingReloads === 0) {
        yield* Effect.yieldNow()
      }
      while ((yield* app.get).activity._tag !== "Idle") {
        yield* Effect.yieldNow()
      }
      const reloadedSnapshot = yield* app.get
      expect(reloadedSnapshot.hideThinkingBlock).toBe(true)
      expect(reloadedSnapshot.activeTools).toEqual(["read", "sandbox"])
      expect(reloadedSnapshot.extensionPaths).toEqual([
        "/agent/extensions/reloaded",
      ])
      expect(
        reloadedSnapshot.commands.some(
          (command) => command.name === "after-reload",
        ),
      ).toBe(true)
      expect(
        reloadedSnapshot.transcript.rows.some(
          (row) =>
            row.content === "Reloaded Pi resources and keybindings",
        ),
      ).toBe(true)

      prompt = ""
      const pastedPath = "/tmp/pi-opentui-paste-1/paste.txt"
      yield* app.dispatch({
        _tag: "Prompt",
        text: pastedPath,
        delivery: "steer",
      })
      while (prompt !== pastedPath) {
        yield* Effect.yieldNow()
      }
      expect(promptCalls).toBe(callsBeforeCommands + 1)
      finishPrompt?.()
      while ((yield* app.get).activity._tag !== "Idle") {
        yield* Effect.yieldNow()
      }

      prompt = ""
      yield* app.dispatch({
        _tag: "Prompt",
        text: "/session",
        delivery: "steer",
      })
      while (prompt !== "/session") {
        yield* Effect.yieldNow()
      }
      expect(promptCalls).toBe(callsBeforeCommands + 2)
      finishPrompt?.()
      while ((yield* app.get).activity._tag !== "Idle") {
        yield* Effect.yieldNow()
      }

      yield* app.dispatch({
        _tag: "RunCommand",
        name: "missing",
        arguments: "",
        delivery: "steer",
      })
      while (
        !(yield* app.get).transcript.rows.some(
          (row) => row.content === "Unknown command: /missing",
        )
      ) {
        yield* Effect.yieldNow()
      }
      expect(promptCalls).toBe(callsBeforeCommands + 2)

      holdCompaction = true
      yield* app.dispatch({
        _tag: "RunCommand",
        name: "compact",
        arguments: "keep decisions",
        delivery: "steer",
      })
      while (compactInstructions === undefined) {
        yield* Effect.yieldNow()
      }
      expect(compactInstructions).toBe("keep decisions")
      emit?.({ type: "compaction_start", reason: "manual" })
      while ((yield* app.get).lastEvent !== "compaction_start") {
        yield* Effect.yieldNow()
      }

      yield* app.dispatch({
        _tag: "Prompt",
        text: "continue after compact",
        delivery: "steer",
      })
      yield* app.dispatch({
        _tag: "Prompt",
        text: "then report",
        delivery: "followUp",
      })
      while ((yield* app.get).promptQueue.followUp.length === 0) {
        yield* Effect.yieldNow()
      }
      expect((yield* app.get).promptQueue).toEqual({
        steering: ["continue after compact"],
        followUp: ["then report"],
      })
      yield* app.dispatch({
        _tag: "RunCommand",
        name: "after-reload",
        arguments: "staged",
        delivery: "steer",
      })
      for (let attempt = 0; attempt < 20; attempt += 1) {
        yield* Effect.yieldNow()
      }
      expect(promptRequests).toContainEqual({
        text: "/after-reload staged",
        delivery: undefined,
      })
      expect((yield* app.get).promptQueue).toEqual({
        steering: ["continue after compact"],
        followUp: ["then report"],
      })

      prompt = ""
      emit?.({
        type: "compaction_end",
        reason: "manual",
        result: undefined,
        aborted: false,
        willRetry: false,
      })
      while (
        prompt !== "continue after compact" ||
        !directQueueRequests.some(
          (request) =>
            request.text === "then report" &&
            request.delivery === "followUp",
        )
      ) {
        yield* Effect.yieldNow()
      }
      finishCompaction?.()
      while ((yield* app.get).activity._tag !== "Turn") {
        yield* Effect.yieldNow()
      }
      finishPrompt?.()
      while ((yield* app.get).activity._tag !== "Idle") {
        yield* Effect.yieldNow()
      }

      compactInstructions = undefined
      finishCompaction = undefined
      failCompaction = undefined
      yield* app.dispatch({
        _tag: "RunCommand",
        name: "compact",
        arguments: "cancel this",
        delivery: "steer",
      })
      while (compactInstructions !== "cancel this") {
        yield* Effect.yieldNow()
      }
      emit?.({ type: "compaction_start", reason: "manual" })
      while ((yield* app.get).lastEvent !== "compaction_start") {
        yield* Effect.yieldNow()
      }
      yield* app.dispatch({
        _tag: "Prompt",
        text: "restore after cancel",
        delivery: "steer",
      })
      while ((yield* app.get).promptQueue.steering.length === 0) {
        yield* Effect.yieldNow()
      }
      const abortsBeforeCompaction = abortCompactionCalls
      yield* app.dispatch({ _tag: "Abort" })
      while (
        abortCompactionCalls === abortsBeforeCompaction ||
        (yield* app.get).activity._tag !== "Idle"
      ) {
        yield* Effect.yieldNow()
      }
      const cancelledRestore = (yield* app.get).draftRestore
      expect(cancelledRestore?.text).toBe("restore after cancel")
      yield* app.dispatch({
        _tag: "AcknowledgeDraftRestore",
        id: cancelledRestore?.id ?? -1,
      })
      while ((yield* app.get).draftRestore !== undefined) {
        yield* Effect.yieldNow()
      }
      holdCompaction = false
      finishCompaction = undefined
      failCompaction = undefined

      yield* app.dispatch({
        _tag: "RunCommand",
        name: "model",
        arguments: "glm",
        delivery: "steer",
      })
      let modelDialog = (yield* app.get).dialog
      while (modelDialog === undefined) {
        yield* Effect.yieldNow()
        modelDialog = (yield* app.get).dialog
      }
      expect(modelDialog.title).toBe("Choose model")
      expect(modelDialog.kind).toBe("search")
      expect(modelDialog.initialQuery).toBe("glm")
      expect((yield* app.get).activity).toEqual({
        _tag: "Command",
        command: "model",
      })
      const opencodeModel = modelDialog.options.find((option) =>
        option.startsWith("opencode-go/glm-5.2 "),
      )
      expect(opencodeModel).toBeDefined()
      yield* app.dispatch({
        _tag: "ResolveDialog",
        id: modelDialog.id,
        value: opencodeModel,
      })
      while (selectedModel === undefined) {
        yield* Effect.yieldNow()
      }
      expect(selectedModel).toBe("opencode-go/glm-5.2")
      expect((yield* app.get).model?.id).toBe("glm-5.2")
      expect((yield* app.get).activity._tag).toBe("Idle")

      yield* app.dispatch({
        _tag: "RunCommand",
        name: "thinking",
        arguments: "",
        delivery: "steer",
      })
      let thinkingDialog = (yield* app.get).dialog
      while (thinkingDialog === undefined) {
        yield* Effect.yieldNow()
        thinkingDialog = (yield* app.get).dialog
      }
      expect(thinkingDialog.title).toBe("Choose thinking level")
      expect((yield* app.get).activity).toEqual({
        _tag: "Command",
        command: "thinking",
      })
      const highThinking = thinkingDialog.options.find((option) =>
        option.startsWith("high "),
      )
      expect(highThinking).toBeDefined()
      yield* app.dispatch({
        _tag: "ResolveDialog",
        id: thinkingDialog.id,
        value: highThinking,
      })
      while (selectedThinking === undefined) {
        yield* Effect.yieldNow()
      }
      expect(selectedThinking).toBe("high")
      expect((yield* app.get).thinkingLevel).toBe("high")
      expect((yield* app.get).activity._tag).toBe("Idle")

      yield* app.dispatch({
        _tag: "RunCommand",
        name: "login",
        arguments: "opencode-go",
        delivery: "steer",
      })
      let loginDialog = (yield* app.get).dialog
      while (loginDialog === undefined) {
        yield* Effect.yieldNow()
        loginDialog = (yield* app.get).dialog
      }
      expect(loginDialog.kind).toBe("secret")
      expect(loginDialog.title).toBe("Enter API key")
      expect((yield* app.get).activity).toEqual({
        _tag: "Command",
        command: "login",
      })
      expect(loginDialog.message).toContain(
        "https://login.example.test/device",
      )
      yield* app.dispatch({
        _tag: "ResolveDialog",
        id: loginDialog.id,
        value: "private-key",
      })
      while (savedLogin === undefined) {
        yield* Effect.yieldNow()
      }
      expect(savedLogin).toBe(
        "opencode-go/api_key/private-key",
      )
      while (
        !(yield* app.get).transcript.rows.some(
          (row) => row.content === "Saved API key for OpenCode Go",
        )
      ) {
        yield* Effect.yieldNow()
      }
      expect(
        (yield* app.get).transcript.rows.some((row) =>
          row.content.includes("private-key"),
        ),
      ).toBe(false)
      expect(
        (yield* app.get).transcript.rows.some((row) =>
          row.content.includes("login.example.test"),
        ),
      ).toBe(false)

      savedLogin = undefined
      yield* app.dispatch({
        _tag: "RunCommand",
        name: "login",
        arguments: "opencode-go",
        delivery: "steer",
      })
      let cancelledDialog = (yield* app.get).dialog
      while (cancelledDialog === undefined) {
        yield* Effect.yieldNow()
        cancelledDialog = (yield* app.get).dialog
      }
      yield* app.dispatch({
        _tag: "ResolveDialog",
        id: cancelledDialog.id,
        value: undefined,
      })
      while ((yield* app.get).activity._tag !== "Idle") {
        yield* Effect.yieldNow()
      }
      expect(activityError((yield* app.get).activity)).toBeUndefined()
      expect(savedLogin).toBeUndefined()

      yield* app.dispatch({
        _tag: "RunCommand",
        name: "login",
        arguments: "opencode-go",
        delivery: "steer",
      })
      while ((yield* app.get).dialog === undefined) {
        yield* Effect.yieldNow()
      }
      yield* app.dispatch({ _tag: "Abort" })
      while ((yield* app.get).activity._tag !== "Idle") {
        yield* Effect.yieldNow()
      }
      expect(activityError((yield* app.get).activity)).toBeUndefined()
      expect(savedLogin).toBeUndefined()

      yield* app.dispatch({
        _tag: "RunCommand",
        name: "new",
        arguments: "",
        delivery: "steer",
      })
      while (
        !(yield* app.get).transcript.rows.some(
          (row) => row.content === "Started a new session",
        )
      ) {
        yield* Effect.yieldNow()
      }
      expect(newSessionCalls).toBe(1)

      yield* app.dispatch({
        _tag: "RunCommand",
        name: "resume",
        arguments: "",
        delivery: "steer",
      })
      let resumeDialog = (yield* app.get).dialog
      while (resumeDialog === undefined) {
        yield* Effect.yieldNow()
        resumeDialog = (yield* app.get).dialog
      }
      expect(resumeDialog.title).toBe("Resume session")
      expect((yield* app.get).activity).toEqual({
        _tag: "Command",
        command: "resume",
      })
      yield* app.dispatch({
        _tag: "ResolveDialog",
        id: resumeDialog.id,
        value: resumeDialog.options[0],
      })
      while (
        !(yield* app.get).transcript.rows.some((row) =>
          row.content.includes("Resumed old-session"),
        )
      ) {
        yield* Effect.yieldNow()
      }
      expect(resumedPath).toBe("/sessions/old.jsonl")
      expect((yield* app.get).activity._tag).toBe("Idle")
      expect(
        (yield* app.get).transcript.rows.map((row) => row.content),
      ).toContain("old answer")

      prompt = ""
      yield* app.dispatch({
        _tag: "Prompt",
        text: "fatal race",
        delivery: "steer",
      })
      while (prompt !== "fatal race") {
        yield* Effect.yieldNow()
      }
      emit({ type: "future_event" } as unknown as AgentSessionEvent)
      while ((yield* app.get).activity._tag !== "Fatal") {
        yield* Effect.yieldNow()
      }
      expect(activityError((yield* app.get).activity)).toBe(
        "Unhandled AgentSessionEvent type: future_event",
      )
      finishPrompt?.()
      for (let index = 0; index < 5; index += 1) {
        yield* Effect.yieldNow()
      }
      expect((yield* app.get).activity._tag).toBe("Fatal")
      const callsBeforeFatalPrompt = promptCalls
      yield* app.dispatch({
        _tag: "Prompt",
        text: "must not run",
        delivery: "steer",
      })
      yield* Effect.yieldNow()
      expect(promptCalls).toBe(callsBeforeFatalPrompt)
      expect((yield* app.get).activity._tag).toBe("Fatal")
    }),
  ).pipe(Effect.provide(appLayer))

  return Effect.runPromise(program)
})
