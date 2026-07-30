import type { PlatformError } from "@effect/platform/Error"
import * as FileSystem from "@effect/platform/FileSystem"
import * as Path from "@effect/platform/Path"
import { Effect } from "effect"

export interface ProjectPath {
  readonly path: string
  readonly isDirectory: boolean
}

export interface ProjectPathOptions {
  readonly maxDepth?: number
  readonly maxEntries?: number
  readonly excludedDirectoryNames?: ReadonlySet<string>
}

const defaultMaxDepth = 24
const defaultMaxEntries = 20_000

export const defaultExcludedDirectoryNames: ReadonlySet<string> =
  new Set([
    ".git",
    ".jj",
    ".direnv",
    ".next",
    ".turbo",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "target",
  ])

const displayPath = (path: string): string =>
  path.replaceAll("\\", "/").replace(/^\.\//, "")

const boundedWholeNumber = (
  value: number | undefined,
  fallback: number,
): number =>
  value === undefined || !Number.isFinite(value)
    ? fallback
    : Math.max(0, Math.floor(value))

/**
 * Build a small project path index without walking known build and cache trees.
 * Nested read and stat faults get skipped so one stale link does not stop the
 * whole index. A fault while reading the root still reaches the caller.
 */
export const listProjectPaths = (
  cwd: string,
  options: ProjectPathOptions = {},
): Effect.Effect<
  ReadonlyArray<ProjectPath>,
  PlatformError,
  FileSystem.FileSystem | Path.Path
> =>
  Effect.gen(function* () {
    const fileSystem = yield* FileSystem.FileSystem
    const path = yield* Path.Path
    const maxDepth = boundedWholeNumber(
      options.maxDepth,
      defaultMaxDepth,
    )
    const maxEntries = boundedWholeNumber(
      options.maxEntries,
      defaultMaxEntries,
    )
    const excluded =
      options.excludedDirectoryNames ??
      defaultExcludedDirectoryNames
    const queue: Array<{
      readonly relative: string
      readonly depth: number
    }> = [{ relative: "", depth: 0 }]
    const results: Array<ProjectPath> = []

    while (queue.length > 0 && results.length < maxEntries) {
      const directory = queue.shift()
      if (directory === undefined) break

      const absoluteDirectory =
        directory.relative.length === 0
          ? cwd
          : path.join(cwd, directory.relative)
      const names =
        directory.relative.length === 0
          ? yield* fileSystem.readDirectory(absoluteDirectory)
          : yield* fileSystem
              .readDirectory(absoluteDirectory)
              .pipe(Effect.catchAll(() => Effect.succeed([])))
      const entries = yield* Effect.forEach(
        names.toSorted((left, right) => left.localeCompare(right)),
        (name) => {
          const relative = displayPath(
            directory.relative.length === 0
              ? name
              : path.join(directory.relative, name),
          )
          return fileSystem.stat(path.join(cwd, relative)).pipe(
            Effect.map((info) => ({
              name,
              relative,
              type: info.type,
            })),
            Effect.catchAll(() => Effect.succeed(undefined)),
          )
        },
        { concurrency: 16 },
      )

      for (const entry of entries) {
        if (entry === undefined) continue
        if (
          entry.type === "Directory" &&
          excluded.has(entry.name)
        ) {
          continue
        }
        if (
          entry.type !== "Directory" &&
          entry.type !== "File"
        ) {
          continue
        }

        const isDirectory = entry.type === "Directory"
        results.push({
          path: `${entry.relative}${isDirectory ? "/" : ""}`,
          isDirectory,
        })
        if (results.length >= maxEntries) break

        if (isDirectory && directory.depth < maxDepth) {
          queue.push({
            relative: entry.relative,
            depth: directory.depth + 1,
          })
        }
      }
    }

    return results.toSorted(
      (left, right) =>
        Number(right.isDirectory) -
          Number(left.isDirectory) ||
        left.path.localeCompare(right.path),
    )
  })
