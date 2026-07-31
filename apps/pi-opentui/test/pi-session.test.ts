import {
  type AgentSessionEvent,
} from "@earendil-works/pi-coding-agent"
import { describe, expect, test } from "bun:test"
import {
  Effect,
  Fiber,
  Layer,
  Option,
  Stream,
} from "effect"
import { AppConfig } from "../src/services/app-config.ts"
import {
  PiSession,
  makePiSessionLayer,
  type OpenedPiSession,
} from "../src/services/pi-session.ts"

describe("PiSession", () => {
  test("delivers events and cleans up its listener and session", () => {
    let listener: ((event: AgentSessionEvent) => void) | undefined
    let disposed = false
    let shutDown = false
    let unsubscribed = false
    let openedWithSavedSessions: boolean | undefined
    let listedSessions = 0
    let newSessions = 0
    let resumedPath: string | undefined
    let selectedModel: string | undefined
    let selectedThinking: string | undefined
    let loginCall: string | undefined
    let reloads = 0

    const open = (
      _cwd: string,
      saveSessions: boolean,
    ): Promise<OpenedPiSession> => {
      openedWithSavedSessions = saveSessions
      return Promise.resolve({
        getHideThinkingBlock: () => true,
        getCommands: () => [],
        getModelState: () => ({
          selected: {
            provider: "openai",
            id: "gpt-5",
            name: "GPT-5",
            reasoning: true,
          },
          thinkingLevel: "medium",
          thinkingLevels: ["off", "medium", "high"],
        }),
        listModels: () =>
          Promise.resolve([
            {
              provider: "openai",
              id: "gpt-5",
              name: "GPT-5",
              reasoning: true,
            },
          ]),
        listAuthProviders: () =>
          Promise.resolve([
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
        listSessions: () => {
          listedSessions += 1
          return Promise.resolve([])
        },
        getMessages: () => [{ role: "user", content: "current" }],
        newSession: () => {
          newSessions += 1
          return Promise.resolve([])
        },
        resume: (path) => {
          resumedPath = path
          return Promise.resolve([
            { role: "user", content: "restored" },
          ])
        },
        selectModel: (provider, id) => {
          selectedModel = `${provider}/${id}`
          return Promise.resolve({
            selected: { provider, id, name: id, reasoning: true },
            thinkingLevel: "medium",
            thinkingLevels: ["off", "medium", "high"],
          })
        },
        selectThinking: (level) => {
          selectedThinking = level
          return Promise.resolve({
            selected: {
              provider: "openai",
              id: "gpt-5",
              name: "GPT-5",
              reasoning: true,
            },
            thinkingLevel: level,
            thinkingLevels: ["off", "medium", "high"],
          })
        },
        login: (provider, type) => {
          loginCall = `${provider}/${type}`
          return Promise.resolve()
        },
        reload: () => {
          reloads += 1
          return Promise.resolve({
            hideThinkingBlock: true,
            activeTools: ["read", "sandbox"],
            extensionPaths: ["/agent/extensions/sandbox"],
            extensionErrors: [],
            commands: [],
            modelState: {
              selected: {
                provider: "openai",
                id: "gpt-5",
                name: "GPT-5",
                reasoning: true,
              },
              thinkingLevel: "medium",
              thinkingLevels: ["off", "medium", "high"],
            },
          })
        },
        shutdown: () => {
          shutDown = true
          return Promise.resolve()
        },
        session: {
          subscribe: (next: (event: AgentSessionEvent) => void) => {
            listener = next
            return () => {
              unsubscribed = true
            }
          },
          dispose: () => {
            disposed = true
          },
          getActiveToolNames: () => ["read", "sandbox"],
          getSessionStats: () => ({
            sessionFile: undefined,
            sessionId: "session-1",
            userMessages: 0,
            assistantMessages: 0,
            toolCalls: 0,
            toolResults: 0,
            totalMessages: 0,
            tokens: {
              input: 0,
              output: 0,
              cacheRead: 0,
              cacheWrite: 0,
              total: 0,
            },
            cost: 0,
          }),
          prompt: () => Promise.resolve(),
          compact: () => Promise.resolve(),
          abort: () => Promise.resolve(),
          bindExtensions: () => Promise.resolve(),
        },
        extensionsResult: {
          extensions: [
            { path: "/agent/extensions/sandbox" },
          ],
          errors: [],
        },
      })
    }

    const config = Layer.succeed(AppConfig, {
      cwd: "/work",
      saveSessions: false,
    })
    const session = makePiSessionLayer(open).pipe(
      Layer.provide(config),
    )

    const program = Effect.scoped(
      Effect.gen(function* () {
        const pi = yield* PiSession
        const waitForEvent = yield* Stream.runHead(pi.events).pipe(
          Effect.fork,
        )

        while (listener === undefined) {
          yield* Effect.yieldNow()
        }
        listener({ type: "agent_settled" })

        const event = yield* Fiber.join(waitForEvent)
        expect(Option.getOrThrow(event).type).toBe("agent_settled")
        expect(pi.activeTools).toEqual(["read", "sandbox"])
        expect(pi.hideThinkingBlock).toBe(true)
        expect(pi.extensionPaths).toEqual([
          "/agent/extensions/sandbox",
        ])
        expect((yield* pi.modelState).thinkingLevel).toBe("medium")
        expect(yield* pi.models).toHaveLength(1)
        expect(yield* pi.authProviders).toHaveLength(1)
        expect(yield* pi.sessions).toEqual([])
        expect(yield* pi.messages).toEqual([
          { role: "user", content: "current" },
        ])
        expect(yield* pi.newSession).toEqual([])
        expect(yield* pi.resume("/sessions/old.jsonl")).toEqual([
          { role: "user", content: "restored" },
        ])
        expect(
          (yield* pi.selectModel("openai", "gpt-5")).selected?.id,
        ).toBe("gpt-5")
        expect((yield* pi.selectThinking("high")).thinkingLevel).toBe(
          "high",
        )
        yield* pi.login("opencode-go", "api_key", {
          prompt: () => Promise.resolve("key"),
          notify: () => undefined,
        })
        expect((yield* pi.reload).hideThinkingBlock).toBe(true)
        expect(openedWithSavedSessions).toBe(false)
        expect(listedSessions).toBe(1)
        expect(newSessions).toBe(1)
        expect(resumedPath).toBe("/sessions/old.jsonl")
        expect(selectedModel).toBe("openai/gpt-5")
        expect(selectedThinking).toBe("high")
        expect(loginCall).toBe("opencode-go/api_key")
        expect(reloads).toBe(1)
      }),
    ).pipe(Effect.provide(session))

    return Effect.runPromise(program).then(() => {
      expect(unsubscribed).toBe(true)
      expect(shutDown).toBe(true)
      expect(disposed).toBe(true)
    })
  })
})
