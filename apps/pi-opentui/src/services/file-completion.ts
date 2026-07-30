import type { ProjectPath } from "./project-paths.ts"

export type { ProjectPath } from "./project-paths.ts"

export interface FileMention {
  readonly start: number
  readonly end: number
  readonly replaceEnd: number
  readonly prefix: string
  readonly query: string
  readonly quoted: boolean
}

export interface FileCompletion extends ProjectPath {
  readonly replacement: string
}

export interface AppliedFileCompletion {
  readonly text: string
  readonly cursorOffset: number
}

const defaultMatchLimit = 8

const displayPath = (path: string): string =>
  path.replaceAll("\\", "/").replace(/^\.\//, "")

const boundedWholeNumber = (
  value: number | undefined,
  fallback: number,
): number =>
  value === undefined || !Number.isFinite(value)
    ? fallback
    : Math.max(0, Math.floor(value))

const isTokenBoundary = (character: string | undefined): boolean =>
  character === undefined ||
  character === " " ||
  character === "\t" ||
  character === "\n" ||
  character === "=" ||
  character === "'" ||
  character === '"'

const safeCursorOffset = (text: string, cursorOffset: number): number =>
  Number.isFinite(cursorOffset)
    ? Math.max(0, Math.min(Math.floor(cursorOffset), text.length))
    : text.length

export const fileMentionAtCursor = (
  text: string,
  cursorOffset = text.length,
): FileMention | undefined => {
  const end = safeCursorOffset(text, cursorOffset)
  const beforeCursor = text.slice(0, end)

  for (let start = beforeCursor.length - 1; start >= 0; start -= 1) {
    if (beforeCursor[start] !== "@") continue
    if (!isTokenBoundary(beforeCursor[start - 1])) continue

    const prefix = beforeCursor.slice(start)
    if (prefix.startsWith('@"')) {
      const query = prefix.slice(2)
      if (query.includes('"')) return undefined
      return {
        start,
        end,
        replaceEnd: (() => {
          const closingQuote = text.indexOf('"', end)
          return closingQuote === -1 ? end : closingQuote + 1
        })(),
        prefix,
        query: displayPath(query),
        quoted: true,
      }
    }
    if (
      prefix
        .slice(1)
        .split("")
        .some((character) => isTokenBoundary(character))
    ) {
      return undefined
    }

    return {
      start,
      end,
      replaceEnd: (() => {
        let tokenEnd = end
        while (
          tokenEnd < text.length &&
          !isTokenBoundary(text[tokenEnd])
        ) {
          tokenEnd += 1
        }
        return tokenEnd
      })(),
      prefix,
      query: displayPath(prefix.slice(1)),
      quoted: false,
    }
  }

  return undefined
}

const pathName = (candidate: ProjectPath): string => {
  const withoutSlash = candidate.isDirectory
    ? candidate.path.slice(0, -1)
    : candidate.path
  return withoutSlash.slice(withoutSlash.lastIndexOf("/") + 1)
}

const matchRank = (
  candidate: ProjectPath,
  query: string,
): number | undefined => {
  if (query.length === 0) return candidate.isDirectory ? 0 : 1

  const path = candidate.path.toLowerCase()
  const name = pathName(candidate).toLowerCase()
  if (path === query || path === `${query}/`) return 0
  if (name === query) return 1
  if (path.startsWith(query)) return 2
  if (name.startsWith(query)) return 3
  if (
    path
      .split("/")
      .some((segment) => segment.startsWith(query))
  ) {
    return 4
  }
  if (name.includes(query)) return 5
  if (path.includes(query)) return 6
  return undefined
}

const completionReplacement = (
  candidate: ProjectPath,
  quoted: boolean,
): string => {
  const needsQuotes = quoted || candidate.path.includes(" ")
  return needsQuotes
    ? `@"${candidate.path}"`
    : `@${candidate.path}`
}

export const fileMentionMatches = (
  candidates: ReadonlyArray<ProjectPath>,
  text: string,
  cursorOffset = text.length,
  limit = defaultMatchLimit,
): ReadonlyArray<FileCompletion> => {
  const mention = fileMentionAtCursor(text, cursorOffset)
  const maximum = boundedWholeNumber(limit, defaultMatchLimit)
  if (mention === undefined || maximum === 0) return []

  const query = mention.query.toLowerCase()
  interface RankedMatch {
    readonly candidate: ProjectPath
    readonly index: number
    readonly rank: number
  }
  const compareMatches = (
    left: RankedMatch,
    right: RankedMatch,
  ): number =>
    left.rank - right.rank ||
    Number(right.candidate.isDirectory) -
      Number(left.candidate.isDirectory) ||
    left.candidate.path.length - right.candidate.path.length ||
    left.candidate.path.localeCompare(right.candidate.path) ||
    left.index - right.index
  const best: Array<RankedMatch> = []
  candidates.forEach((candidate, index) => {
    const rank = matchRank(candidate, query)
    if (rank === undefined) return
    best.push({ candidate, index, rank })
    best.sort(compareMatches)
    if (best.length > maximum) best.pop()
  })

  return best
    .map(({ candidate }) => ({
      ...candidate,
      replacement: completionReplacement(
        candidate,
        mention.quoted,
      ),
    }))
}

export const applyFileMentionCompletion = (
  text: string,
  cursorOffset: number,
  completion: FileCompletion,
): AppliedFileCompletion | undefined => {
  const mention = fileMentionAtCursor(text, cursorOffset)
  if (mention === undefined) return undefined

  let after = text.slice(mention.replaceEnd)
  if (
    completion.replacement.endsWith('"') &&
    after.startsWith('"')
  ) {
    after = after.slice(1)
  }
  const suffix = completion.isDirectory
    ? ""
    : after.length === 0 || !/^\s/.test(after)
      ? " "
      : ""
  const nextText =
    text.slice(0, mention.start) +
    completion.replacement +
    suffix +
    after
  const quotedDirectory =
    completion.isDirectory &&
    completion.replacement.endsWith('"')
  const nextCursor =
    mention.start +
    completion.replacement.length +
    suffix.length -
    (quotedDirectory ? 1 : 0)

  return { text: nextText, cursorOffset: nextCursor }
}
