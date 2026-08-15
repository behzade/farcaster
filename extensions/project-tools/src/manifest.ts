import { Effect } from "effect";
import { readFile, lstat, realpath } from "node:fs/promises";
import { isAbsolute, relative, resolve } from "node:path";
import type { TSchema } from "typebox";
import type { Validator } from "typebox/compile";
import { ProjectToolLoadError } from "./errors.ts";
import { compileStrictSchema } from "./schema.ts";

export interface ProjectToolManifest {
  readonly version: 1;
  readonly name: string;
  readonly label: string;
  readonly description: string;
  readonly entrypoint: string;
  readonly parameters: TSchema;
  readonly result: TSchema;
}

export interface LoadedManifest {
  readonly directory: string;
  readonly entrypoint: string;
  readonly manifest: ProjectToolManifest;
  readonly parametersValidator: Validator;
  readonly resultValidator: Validator;
}

const MANIFEST_KEYS = new Set(["version", "name", "label", "description", "entrypoint", "parameters", "result"]);
const TOOL_NAME = /^[a-z][a-z0-9_]{0,63}$/;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

function parseManifest(text: string, manifestPath: string): LoadedManifest["manifest"] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch (cause) {
    throw new ProjectToolLoadError({ path: manifestPath, message: "must contain strict JSON", cause });
  }
  if (!isRecord(parsed)) throw new ProjectToolLoadError({ path: manifestPath, message: "must contain a JSON object" });
  for (const key of Object.keys(parsed)) {
    if (!MANIFEST_KEYS.has(key)) throw new ProjectToolLoadError({ path: `${manifestPath}.${key}`, message: "unknown manifest field" });
  }
  if (parsed.version !== 1) throw new ProjectToolLoadError({ path: `${manifestPath}.version`, message: "must be 1" });
  if (typeof parsed.name !== "string" || !TOOL_NAME.test(parsed.name)) {
    throw new ProjectToolLoadError({ path: `${manifestPath}.name`, message: "must match ^[a-z][a-z0-9_]{0,63}$" });
  }
  if (typeof parsed.label !== "string" || parsed.label.length < 1 || parsed.label.length > 80) {
    throw new ProjectToolLoadError({ path: `${manifestPath}.label`, message: "must contain 1 to 80 characters" });
  }
  if (typeof parsed.description !== "string" || parsed.description.length < 1 || parsed.description.length > 1000) {
    throw new ProjectToolLoadError({ path: `${manifestPath}.description`, message: "must contain 1 to 1000 characters" });
  }
  if (typeof parsed.entrypoint !== "string" || !parsed.entrypoint.endsWith(".ts")) {
    throw new ProjectToolLoadError({ path: `${manifestPath}.entrypoint`, message: "must name a TypeScript file" });
  }
  return parsed as unknown as ProjectToolManifest;
}

const attempt = <A>(path: string, message: string, operation: () => Promise<A>) =>
  Effect.tryPromise({
    try: operation,
    catch: (cause) => new ProjectToolLoadError({ path, message, cause }),
  });
export const loadManifest = (directory: string) =>
  Effect.gen(function*() {
    const manifestPath = resolve(directory, "tool.json");
    const manifestStat = yield* attempt(manifestPath, "could not read manifest metadata", () => lstat(manifestPath));
    if (!manifestStat.isFile() || manifestStat.isSymbolicLink()) {
      return yield* new ProjectToolLoadError({ path: manifestPath, message: "must be a regular file" });
    }
    const text = yield* attempt(manifestPath, "could not read manifest", () => readFile(manifestPath, "utf8"));
    const manifest = yield* Effect.try({
      try: () => parseManifest(text, manifestPath),
      catch: (cause) => cause instanceof ProjectToolLoadError
        ? cause
        : new ProjectToolLoadError({ path: manifestPath, message: "could not parse manifest", cause }),
    });
    if (manifest.name !== directory.split("/").at(-1)) {
      return yield* new ProjectToolLoadError({ path: `${manifestPath}.name`, message: "must match its tool directory name" });
    }

    const entrypoint = resolve(directory, manifest.entrypoint);
    const relativeEntrypoint = relative(directory, entrypoint);
    if (isAbsolute(relativeEntrypoint) || relativeEntrypoint === ".." || relativeEntrypoint.startsWith(`..${process.platform === "win32" ? "\\" : "/"}`)) {
      return yield* new ProjectToolLoadError({ path: `${manifestPath}.entrypoint`, message: "must stay inside the tool directory" });
    }
    const entrypointStat = yield* attempt(entrypoint, "could not read entrypoint metadata", () => lstat(entrypoint));
    if (!entrypointStat.isFile() || entrypointStat.isSymbolicLink()) {
      return yield* new ProjectToolLoadError({ path: entrypoint, message: "must be a regular file" });
    }
    const canonicalDirectory = yield* attempt(directory, "could not resolve tool directory", () => realpath(directory));
    const canonicalEntrypoint = yield* attempt(entrypoint, "could not resolve entrypoint", () => realpath(entrypoint));
    const canonicalRelative = relative(canonicalDirectory, canonicalEntrypoint);
    if (isAbsolute(canonicalRelative) || canonicalRelative === ".." || canonicalRelative.startsWith(`..${process.platform === "win32" ? "\\" : "/"}`)) {
      return yield* new ProjectToolLoadError({ path: entrypoint, message: "must not resolve outside the tool directory" });
    }

    const schemaBytes = Buffer.byteLength(JSON.stringify([manifest.parameters, manifest.result]), "utf8");
    if (schemaBytes > 64 * 1024) {
      return yield* new ProjectToolLoadError({ path: manifestPath, message: "parameter and result schemas may total at most 64 KiB" });
    }
    const parametersValidator = yield* Effect.try({
      try: () => compileStrictSchema(manifest.parameters, `${manifestPath}.parameters`, true),
      catch: (cause) => cause instanceof ProjectToolLoadError
        ? cause
        : new ProjectToolLoadError({ path: `${manifestPath}.parameters`, message: "invalid parameter schema", cause }),
    });
    const resultValidator = yield* Effect.try({
      try: () => compileStrictSchema(manifest.result, `${manifestPath}.result`),
      catch: (cause) => cause instanceof ProjectToolLoadError
        ? cause
        : new ProjectToolLoadError({ path: `${manifestPath}.result`, message: "invalid result schema", cause }),
    });

    return {
      directory: canonicalDirectory,
      entrypoint: canonicalEntrypoint,
      manifest,
      parametersValidator,
      resultValidator,
    } satisfies LoadedManifest;
  });
