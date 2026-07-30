import { testRender } from "@opentui/solid"
import { expect, test } from "bun:test"
import { Effect } from "effect"
import { App, type AppBridge } from "../src/app.tsx"

const bridge: AppBridge = {
  initial: {
    cwd: "/work/pi",
    phase: "ready",
    activeTools: ["read", "sandbox"],
    extensionPaths: ["/agent/extensions/sandbox"],
    extensionErrors: [],
    eventCount: 3,
    lastEvent: "agent_settled",
    error: undefined,
  },
  subscribe: () => () => undefined,
  dispatch: () => undefined,
  quit: () => undefined,
}

test("renders Pi SDK and extension status", () =>
  Effect.runPromise(
    Effect.acquireUseRelease(
      Effect.tryPromise(() =>
        testRender(() => <App bridge={bridge} />, {
          width: 100,
          height: 30,
        }),
      ),
      (setup) =>
        Effect.gen(function* () {
          yield* Effect.tryPromise(() => setup.renderOnce())
          const frame = setup.captureCharFrame()

          expect(frame).toContain("pi-next")
          expect(frame).toContain("/work/pi")
          expect(frame).toContain("sandbox")
          expect(frame).toContain("last agent_settled")
        }),
      (setup) => Effect.sync(() => setup.renderer.destroy()),
    ),
  ),
)
