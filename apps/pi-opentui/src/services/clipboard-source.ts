import * as Command from "@effect/platform/Command"
import type { CommandExecutor } from "@effect/platform/CommandExecutor"
import { Data, Duration, Effect, Stream } from "effect"

export type ClipboardContent =
  | {
      readonly kind: "image"
      readonly data: Uint8Array
      readonly mimeType: string
    }
  | { readonly kind: "text"; readonly data: string }

export class ClipboardSourceError extends Data.TaggedError(
  "ClipboardSourceError",
)<{
  readonly source: "native" | "wl-paste" | "xclip"
  readonly cause: unknown
}> {}

const imageMimeTypes = [
  "image/png",
  "image/jpeg",
  "image/webp",
  "image/gif",
] as const

export const maxClipboardReadBytes = 50 * 1024 * 1024

export const selectClipboardImageMimeType = (
  types: string,
): string | undefined => {
  const available = new Set(
    types
      .split(/\r?\n/)
      .map((value) => value.trim().split(";")[0]?.toLowerCase())
      .filter((value): value is string => value !== undefined),
  )
  return imageMimeTypes.find((mimeType) => available.has(mimeType))
}

const combineBytes = (chunks: ReadonlyArray<Uint8Array>): Uint8Array => {
  const length = chunks.reduce((total, chunk) => total + chunk.byteLength, 0)
  const result = new Uint8Array(length)
  let offset = 0
  for (const chunk of chunks) {
    result.set(chunk, offset)
    offset += chunk.byteLength
  }
  return result
}

const commandBytes = (
  executor: CommandExecutor,
  source: ClipboardSourceError["source"],
  command: Command.Command,
): Effect.Effect<Uint8Array, ClipboardSourceError> =>
  executor.stream(command).pipe(
    Stream.runFoldEffect(
      { chunks: [] as Array<Uint8Array>, bytes: 0 },
      (state, chunk) => {
        const bytes = state.bytes + chunk.byteLength
        if (bytes > maxClipboardReadBytes) {
          return Effect.fail(
            new ClipboardSourceError({
              source,
              cause: "Clipboard data exceeds 50 MiB",
            }),
          )
        }
        state.chunks.push(chunk)
        return Effect.succeed({ chunks: state.chunks, bytes })
      },
    ),
    Effect.timeout(Duration.seconds(3)),
    Effect.map(({ chunks }) => combineBytes(chunks)),
    Effect.mapError((cause) =>
      cause instanceof ClipboardSourceError
        ? cause
        : new ClipboardSourceError({ source, cause }),
    ),
  )

const commandString = (
  executor: CommandExecutor,
  source: ClipboardSourceError["source"],
  command: Command.Command,
): Effect.Effect<string, ClipboardSourceError> =>
  commandBytes(executor, source, command).pipe(
    Effect.map((bytes) => new TextDecoder().decode(bytes)),
  )

const readWlPaste = (
  executor: CommandExecutor,
): Effect.Effect<ClipboardContent | undefined, ClipboardSourceError> =>
  Effect.gen(function* () {
    const types = yield* commandString(
      executor,
      "wl-paste",
      Command.make("wl-paste", "--list-types"),
    )
    const imageType = selectClipboardImageMimeType(types)
    if (imageType !== undefined) {
      const data = yield* commandBytes(
        executor,
        "wl-paste",
        Command.make("wl-paste", "--type", imageType, "--no-newline"),
      )
      return data.byteLength === 0
        ? undefined
        : { kind: "image" as const, data, mimeType: imageType }
    }
    const text = yield* commandString(
      executor,
      "wl-paste",
      Command.make("wl-paste", "--no-newline"),
    )
    return text.length === 0 ? undefined : { kind: "text" as const, data: text }
  })

const readXclip = (
  executor: CommandExecutor,
): Effect.Effect<ClipboardContent | undefined, ClipboardSourceError> =>
  Effect.gen(function* () {
    const types = yield* commandString(
      executor,
      "xclip",
      Command.make("xclip", "-selection", "clipboard", "-t", "TARGETS", "-o"),
    )
    const imageType = selectClipboardImageMimeType(types)
    if (imageType !== undefined) {
      const data = yield* commandBytes(
        executor,
        "xclip",
        Command.make(
          "xclip",
          "-selection",
          "clipboard",
          "-t",
          imageType,
          "-o",
        ),
      )
      return data.byteLength === 0
        ? undefined
        : { kind: "image" as const, data, mimeType: imageType }
    }
    const text = yield* commandString(
      executor,
      "xclip",
      Command.make("xclip", "-selection", "clipboard", "-o"),
    )
    return text.length === 0 ? undefined : { kind: "text" as const, data: text }
  })

const readNativeClipboard: Effect.Effect<
  ClipboardContent | undefined,
  ClipboardSourceError
> = Effect.tryPromise({
  try: () => import("@mariozechner/clipboard"),
  catch: (cause) => new ClipboardSourceError({ source: "native", cause }),
}).pipe(
  Effect.flatMap((clipboard) =>
    Effect.try({
      try: () => clipboard.hasImage(),
      catch: (cause) => new ClipboardSourceError({ source: "native", cause }),
    }).pipe(
      Effect.flatMap((hasImage): Effect.Effect<
        ClipboardContent | undefined,
        ClipboardSourceError
      > => {
        if (hasImage) {
          return Effect.tryPromise({
            try: () => clipboard.getImageBinary(),
            catch: (cause) =>
              new ClipboardSourceError({ source: "native", cause }),
          }).pipe(
            Effect.flatMap((bytes) =>
              bytes.length > maxClipboardReadBytes
                ? Effect.fail(
                    new ClipboardSourceError({
                      source: "native",
                      cause: "Clipboard data exceeds 50 MiB",
                    }),
                  )
                : Effect.succeed({
                    kind: "image" as const,
                    data: Uint8Array.from(bytes),
                    mimeType: "image/png",
                  }),
            ),
          )
        }
        return Effect.try({
          try: () => clipboard.hasText(),
          catch: (cause) =>
            new ClipboardSourceError({ source: "native", cause }),
        }).pipe(
          Effect.flatMap((hasText) =>
            hasText
              ? Effect.tryPromise({
                  try: () => clipboard.getText(),
                  catch: (cause) =>
                    new ClipboardSourceError({ source: "native", cause }),
                }).pipe(
                  Effect.flatMap((data) =>
                    data.length > maxClipboardReadBytes
                      ? Effect.fail(
                          new ClipboardSourceError({
                            source: "native",
                            cause: "Clipboard data exceeds 50 MiB",
                          }),
                        )
                      : Effect.succeed({ kind: "text" as const, data }),
                  ),
                )
              : Effect.succeed(undefined),
          ),
        )
      }),
    ),
  ),
)

const firstContent = (
  sources: ReadonlyArray<
    Effect.Effect<ClipboardContent | undefined, ClipboardSourceError>
  >,
): Effect.Effect<ClipboardContent | undefined, never> => {
  const [source, ...rest] = sources
  if (source === undefined) return Effect.succeed(undefined)
  return source.pipe(
    Effect.catchAll(() => Effect.succeed(undefined)),
    Effect.flatMap((content) =>
      content === undefined ? firstContent(rest) : Effect.succeed(content),
    ),
  )
}

export const readSystemClipboard = (
  executor: CommandExecutor,
  env: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
): Effect.Effect<ClipboardContent | undefined> => {
  const sources = {
    native: readNativeClipboard,
    "wl-paste": readWlPaste(executor),
    xclip: readXclip(executor),
  }
  return firstContent(
    clipboardSourceOrder(env, platform).map((source) => sources[source]),
  )
}

export type ClipboardSourceName = "native" | "wl-paste" | "xclip"

export const clipboardSourceOrder = (
  env: NodeJS.ProcessEnv,
  platform: NodeJS.Platform,
): ReadonlyArray<ClipboardSourceName> => {
  if (env.TERMUX_VERSION) return []
  if (platform !== "linux") return ["native"]
  return Boolean(env.WAYLAND_DISPLAY) || env.XDG_SESSION_TYPE === "wayland"
    ? ["wl-paste", "xclip", "native"]
    : ["native", "xclip"]
}
