import * as FileSystem from "@effect/platform/FileSystem"
import { BunContext } from "@effect/platform-bun"
import { expect, test } from "bun:test"
import { Effect } from "effect"
import {
  applyFileMentionCompletion,
  fileMentionAtCursor,
  fileMentionMatches,
  type ProjectPath,
} from "../src/services/file-completion.ts"
import { listProjectPaths } from "../src/services/project-paths.ts"

const candidates: ReadonlyArray<ProjectPath> = [
  { path: "src/", isDirectory: true },
  { path: "src/app.tsx", isDirectory: false },
  { path: "src/services/app-state.ts", isDirectory: false },
  { path: "docs/user guide.md", isDirectory: false },
  { path: "test/app.test.tsx", isDirectory: false },
]

test("finds plain and quoted mentions only at token boundaries", () => {
  expect(fileMentionAtCursor("read @src/ap")).toEqual({
    start: 5,
    end: 12,
    replaceEnd: 12,
    prefix: "@src/ap",
    query: "src/ap",
    quoted: false,
  })
  expect(fileMentionAtCursor('read @"docs/user g')).toEqual({
    start: 5,
    end: 18,
    replaceEnd: 18,
    prefix: '@"docs/user g',
    query: "docs/user g",
    quoted: true,
  })
  expect(fileMentionAtCursor("mail a@src/app")).toBeUndefined()
  expect(fileMentionAtCursor('read @"done.md" next')).toBeUndefined()
})

test("uses the cursor offset instead of assuming the end of the draft", () => {
  const text = "read @src/ap then"
  expect(fileMentionAtCursor(text, 12)?.query).toBe("src/ap")
  expect(
    fileMentionMatches(candidates, text, 12).map(
      (candidate) => candidate.path,
    ),
  ).toEqual(["src/app.tsx"])
})

test("ranks path and file name matches and quotes spaces", () => {
  expect(
    fileMentionMatches(candidates, "@app").map(
      (candidate) => candidate.path,
    ),
  ).toEqual([
    "src/app.tsx",
    "test/app.test.tsx",
    "src/services/app-state.ts",
  ])
  expect(fileMentionMatches(candidates, "@user")).toEqual([
    {
      path: "docs/user guide.md",
      isDirectory: false,
      replacement: '@"docs/user guide.md"',
    },
  ])
  expect(fileMentionMatches(candidates, "@", 1, 2)).toHaveLength(2)
  expect(fileMentionMatches(candidates, "@src", 4, 0)).toEqual([])
})

test("applies file and directory completions without losing draft text", () => {
  const file = fileMentionMatches(candidates, "read @user")[0]
  expect(file).toBeDefined()
  expect(
    applyFileMentionCompletion("read @user", 10, file!),
  ).toEqual({
    text: 'read @"docs/user guide.md" ',
    cursorOffset: 27,
  })

  const directory = fileMentionMatches(candidates, "read @sr")[0]
  expect(directory).toEqual({
    path: "src/",
    isDirectory: true,
    replacement: "@src/",
  })
  expect(
    applyFileMentionCompletion(
      "read @sr after",
      8,
      directory!,
    ),
  ).toEqual({
    text: "read @src/ after",
    cursorOffset: 10,
  })
})

test("replaces the whole mention when the cursor is inside it", () => {
  const text = "read @app.tsx after"
  const completion = fileMentionMatches(candidates, text, 9)[0]
  expect(completion?.path).toBe("src/app.tsx")
  expect(applyFileMentionCompletion(text, 9, completion!)).toEqual({
    text: "read @src/app.tsx after",
    cursorOffset: 17,
  })
})

test("lists project files while skipping large and hidden data trees", () =>
  Effect.runPromise(
    Effect.scoped(
      Effect.gen(function* () {
        const fileSystem = yield* FileSystem.FileSystem
        const root = yield* fileSystem.makeTempDirectoryScoped({
          prefix: "pi-file-completion-",
        })
        yield* fileSystem.makeDirectory(`${root}/src`, {
          recursive: true,
        })
        yield* fileSystem.makeDirectory(`${root}/node_modules/pkg`, {
          recursive: true,
        })
        yield* fileSystem.makeDirectory(`${root}/empty`, {
          recursive: true,
        })
        yield* fileSystem.writeFileString(
          `${root}/src/app.ts`,
          "export {}",
        )
        yield* fileSystem.writeFileString(
          `${root}/node_modules/pkg/index.js`,
          "module.exports = {}",
        )

        expect(yield* listProjectPaths(root)).toEqual([
          { path: "empty/", isDirectory: true },
          { path: "src/", isDirectory: true },
          { path: "src/app.ts", isDirectory: false },
        ])
      }),
    ).pipe(Effect.provide(BunContext.layer)),
  ))

test("caps the project index before walking queued directories", () =>
  Effect.runPromise(
    Effect.scoped(
      Effect.gen(function* () {
        const fileSystem = yield* FileSystem.FileSystem
        const root = yield* fileSystem.makeTempDirectoryScoped({
          prefix: "pi-file-completion-limit-",
        })
        yield* fileSystem.makeDirectory(`${root}/nested`, {
          recursive: true,
        })
        yield* fileSystem.writeFileString(`${root}/one.txt`, "1")
        yield* fileSystem.writeFileString(
          `${root}/nested/two.txt`,
          "2",
        )

        const paths = yield* listProjectPaths(root, {
          maxEntries: 1,
        })
        expect(paths).toHaveLength(1)
        expect(paths[0]?.path).not.toBe("nested/two.txt")
      }),
    ).pipe(Effect.provide(BunContext.layer)),
  ))
