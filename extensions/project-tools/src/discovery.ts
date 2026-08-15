import { Effect, Result } from "effect";
import { lstat, readdir, realpath } from "node:fs/promises";
import { basename, join } from "node:path";
import { ProjectToolLoadError } from "./errors.ts";
import { loadManifest } from "./manifest.ts";
import { loadProjectToolModule, type LoadedProjectTool } from "./module.ts";

export interface ProjectToolDiagnostic {
  readonly tool: string;
  readonly message: string;
}

export interface ProjectToolDiscovery {
  readonly projectRoot: string;
  readonly tools: readonly LoadedProjectTool[];
  readonly diagnostics: readonly ProjectToolDiagnostic[];
}

const MAX_PROJECT_TOOLS = 32;

const attempt = <A>(path: string, message: string, operation: () => Promise<A>) =>
  Effect.tryPromise({
    try: operation,
    catch: (cause) => new ProjectToolLoadError({ path, message, cause }),
  });
export const discoverProjectTools = (projectRoot: string) =>
  Effect.gen(function*() {
    const canonicalRoot = yield* attempt(projectRoot, "could not resolve project root", () => realpath(projectRoot));
    const toolsRoot = join(canonicalRoot, ".pi", "project-tools");
    const rootStat = yield* Effect.result(attempt(toolsRoot, "could not read project tools directory", () => lstat(toolsRoot)));
    if (Result.isFailure(rootStat)) {
      const cause = rootStat.failure.cause as NodeJS.ErrnoException | undefined;
      if (cause?.code === "ENOENT") return { projectRoot: canonicalRoot, tools: [], diagnostics: [] } satisfies ProjectToolDiscovery;
      return {
        projectRoot: canonicalRoot,
        tools: [],
        diagnostics: [{ tool: ".pi/project-tools", message: rootStat.failure.message }],
      } satisfies ProjectToolDiscovery;
    }
    if (!rootStat.success.isDirectory() || rootStat.success.isSymbolicLink()) {
      return {
        projectRoot: canonicalRoot,
        tools: [],
        diagnostics: [{ tool: ".pi/project-tools", message: "must be a real directory" }],
      } satisfies ProjectToolDiscovery;
    }

    const entries = yield* attempt(toolsRoot, "could not list project tools", () => readdir(toolsRoot, { withFileTypes: true }));
    const tools: LoadedProjectTool[] = [];
    const diagnostics: ProjectToolDiagnostic[] = [];
    for (const entry of entries.toSorted((left, right) => left.name.localeCompare(right.name))) {
      if (!entry.isDirectory() || entry.isSymbolicLink()) {
        diagnostics.push({ tool: entry.name, message: "tool entries must be real directories" });
        continue;
      }
      if (tools.length >= MAX_PROJECT_TOOLS) {
        diagnostics.push({ tool: entry.name, message: `at most ${MAX_PROJECT_TOOLS} project tools may be active` });
        continue;
      }
      const loaded = yield* Effect.result(loadManifest(join(toolsRoot, entry.name)).pipe(Effect.flatMap(loadProjectToolModule)));
      if (Result.isSuccess(loaded)) {
        tools.push(loaded.success);
      } else {
        diagnostics.push({ tool: basename(entry.name), message: `${loaded.failure.path}: ${loaded.failure.message}` });
      }
    }
    return { projectRoot: canonicalRoot, tools, diagnostics } satisfies ProjectToolDiscovery;
  });
