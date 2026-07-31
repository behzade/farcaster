import { BunContext } from "@effect/platform-bun"
import * as FileSystem from "@effect/platform/FileSystem"
import { expect, test } from "bun:test"
import { Effect } from "effect"
import { isLargePaste } from "../src/services/paste-model.ts"
import {
  defaultPasteLimits,
  makePasteServiceLayer,
  PasteService,
} from "../src/services/paste-service.ts"

test("keeps small text inline and stores large text in a scoped file", async () => {
  expect(isLargePaste("x".repeat(1_000))).toBe(false)
  expect(isLargePaste("x".repeat(1_001))).toBe(true)
  expect(isLargePaste(Array.from({ length: 10 }, () => "x").join("\n"))).toBe(false)
  expect(isLargePaste(Array.from({ length: 11 }, () => "x").join("\n"))).toBe(true)

  const content = "large\n".repeat(11)
  const result = await Effect.runPromise(
    Effect.scoped(
      Effect.gen(function* () {
        const paste = yield* PasteService
        const fileSystem = yield* FileSystem.FileSystem
        const small = yield* paste.resolve({ kind: "text", text: "small" })
        const large = yield* paste.resolve({ kind: "text", text: content })
        if (large === undefined) throw new Error("Missing paste result")
        return {
          small,
          large,
          stored: yield* fileSystem.readFileString(large.text),
        }
      }).pipe(
        Effect.provide(
          makePasteServiceLayer(Effect.succeed(undefined)),
        ),
        Effect.provide(BunContext.layer),
      ),
    ),
  )

  expect(result.small).toEqual({ kind: "inline", text: "small" })
  expect(result.large.kind).toBe("file")
  expect(result.large.text).toEndWith(".txt")
  expect(result.stored).toBe(content)
  expect(await Bun.file(result.large.text).exists()).toBe(false)
})

test("stores a clipboard image as a scoped png", async () => {
  const bytes = Uint8Array.from([137, 80, 78, 71])
  const result = await Effect.runPromise(
    Effect.scoped(
      Effect.gen(function* () {
        const paste = yield* PasteService
        const fileSystem = yield* FileSystem.FileSystem
        const insertion = yield* paste.resolve({ kind: "clipboard" })
        if (insertion === undefined) throw new Error("Missing image result")
        return {
          insertion,
          stored: yield* fileSystem.readFile(insertion.text),
        }
      }).pipe(
        Effect.provide(
          makePasteServiceLayer(
            Effect.succeed({
              kind: "image",
              data: bytes,
              mimeType: "image/png",
            }),
          ),
        ),
        Effect.provide(BunContext.layer),
      ),
    ),
  )

  expect(result.insertion.kind).toBe("file")
  expect(result.insertion.text).toEndWith(".png")
  expect(result.stored).toEqual(bytes)
  expect(await Bun.file(result.insertion.text).exists()).toBe(false)
})

test("keeps the clipboard image MIME type in its file name", async () => {
  const result = await Effect.runPromise(
    Effect.scoped(
      PasteService.pipe(
        Effect.flatMap((paste) => paste.resolve({ kind: "clipboard" })),
        Effect.provide(
          makePasteServiceLayer(
            Effect.succeed({
              kind: "image",
              data: Uint8Array.from([255, 216, 255]),
              mimeType: "image/jpeg",
            }),
          ),
        ),
        Effect.provide(BunContext.layer),
      ),
    ),
  )

  expect(result?.kind).toBe("file")
  expect(result?.text).toEndWith(".jpg")
  if (result !== undefined) {
    expect(await Bun.file(result.text).exists()).toBe(false)
  }
})

test("rejects files above the per-file storage limit", async () => {
  const result = await Effect.runPromise(
    Effect.scoped(
      PasteService.pipe(
        Effect.flatMap((paste) => paste.resolve({ kind: "clipboard" })),
        Effect.flip,
        Effect.provide(
          makePasteServiceLayer(
            Effect.succeed({
              kind: "image",
              data: Uint8Array.from([1, 2, 3, 4]),
              mimeType: "image/png",
            }),
            { ...defaultPasteLimits, maxFileBytes: 3 },
          ),
        ),
        Effect.provide(BunContext.layer),
      ),
    ),
  )

  expect(result.operation).toBe("limit")
})
