import * as FileSystem from "@effect/platform/FileSystem"
import * as Path from "@effect/platform/Path"
import * as CommandExecutor from "@effect/platform/CommandExecutor"
import { Context, Data, Effect, Layer, Ref, Scope } from "effect"
import {
  readSystemClipboard,
  type ClipboardContent,
} from "./clipboard-source.ts"
import {
  isLargePaste,
  type PasteInsertion,
  type PasteRequest,
} from "./paste-model.ts"

export type ReadClipboard = Effect.Effect<
  ClipboardContent | undefined,
  PasteServiceError
>

export interface PasteServiceShape {
  readonly resolve: (
    request: PasteRequest,
  ) => Effect.Effect<PasteInsertion | undefined, PasteServiceError>
}

export class PasteService extends Context.Tag("pi-opentui/PasteService")<
  PasteService,
  PasteServiceShape
>() {}

export class PasteServiceError extends Data.TaggedError("PasteServiceError")<{
  readonly operation: "load" | "read" | "store" | "limit"
  readonly cause: unknown
}> {}

export interface PasteLimits {
  readonly maxFileBytes: number
  readonly maxFiles: number
  readonly maxTotalBytes: number
}

export const defaultPasteLimits: PasteLimits = {
  maxFileBytes: 50 * 1024 * 1024,
  maxFiles: 64,
  maxTotalBytes: 200 * 1024 * 1024,
}

const imageExtension = (mimeType: string): string => {
  switch (mimeType.toLowerCase()) {
    case "image/jpeg":
      return "jpg"
    case "image/webp":
      return "webp"
    case "image/gif":
      return "gif"
    default:
      return "png"
  }
}

export const makePasteServiceLayer = (
  readClipboard?: ReadClipboard,
  limits: PasteLimits = defaultPasteLimits,
): Layer.Layer<
  PasteService,
  PasteServiceError,
  FileSystem.FileSystem | Path.Path | CommandExecutor.CommandExecutor
> =>
  Layer.scoped(
    PasteService,
    Effect.gen(function* () {
      const fileSystem = yield* FileSystem.FileSystem
      const path = yield* Path.Path
      const commandExecutor = yield* CommandExecutor.CommandExecutor
      const scope = yield* Effect.scope
      const clipboard =
        readClipboard ??
        readSystemClipboard(commandExecutor).pipe(
          Effect.mapError(
            (cause) => new PasteServiceError({ operation: "read", cause }),
          ),
        )
      const tempDirectory = yield* Effect.cached(
        fileSystem
          .makeTempDirectoryScoped({ prefix: "pi-opentui-paste-" })
          .pipe(
            Effect.mapError(
              (cause) =>
                new PasteServiceError({ operation: "store", cause }),
            ),
            Effect.provideService(Scope.Scope, scope),
          ),
      )
      const usage = yield* Ref.make({ files: 0, bytes: 0 })

      const reserve = (bytes: number): Effect.Effect<void, PasteServiceError> =>
        Ref.modify(usage, (current) => {
          const overLimit =
            bytes > limits.maxFileBytes ||
            current.files >= limits.maxFiles ||
            current.bytes + bytes > limits.maxTotalBytes
          return overLimit
            ? [false, current] as const
            : [
                true,
                { files: current.files + 1, bytes: current.bytes + bytes },
              ] as const
        }).pipe(
          Effect.flatMap((reserved) =>
            reserved
              ? Effect.void
              : Effect.fail(
                  new PasteServiceError({
                    operation: "limit",
                    cause: "Paste storage limit exceeded",
                  }),
                ),
          ),
        )

      const release = (bytes: number): Effect.Effect<void> =>
        Ref.update(usage, (current) => ({
          files: Math.max(0, current.files - 1),
          bytes: Math.max(0, current.bytes - bytes),
        }))

      const store = (
        data: Uint8Array,
        fileName: string,
      ): Effect.Effect<PasteInsertion, PasteServiceError> =>
        Effect.gen(function* () {
          yield* reserve(data.byteLength)
          const directory = yield* tempDirectory
          const filePath = path.join(directory, fileName)
          yield* fileSystem.writeFile(filePath, data).pipe(
            Effect.mapError(
              (cause) =>
                new PasteServiceError({ operation: "store", cause }),
            ),
          )
          return { kind: "file" as const, text: filePath }
        }).pipe(Effect.tapError(() => release(data.byteLength)))

      const resolveText = (
        text: string,
      ): Effect.Effect<PasteInsertion, PasteServiceError> =>
        isLargePaste(text)
          ? store(
              new TextEncoder().encode(text),
              `pi-paste-${crypto.randomUUID()}.txt`,
            )
          : Effect.succeed({ kind: "inline", text })

      return {
        resolve: (request) => {
          if (request.kind === "text") return resolveText(request.text)
          return clipboard.pipe(
            Effect.flatMap((content) => {
              if (content === undefined) return Effect.succeed(undefined)
              if (content.kind === "text") {
                return resolveText(content.data)
              }
              return store(
                content.data,
                `pi-clipboard-${crypto.randomUUID()}.${imageExtension(content.mimeType)}`,
              )
            }),
          )
        },
      }
    }),
  )

export const PasteServiceLive = makePasteServiceLayer()
