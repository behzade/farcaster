import { Cause, Effect, Exit, Layer, Option } from "effect";
import { createJiti } from "jiti";
import { fileURLToPath } from "node:url";
import type { LoadedManifest } from "./manifest.ts";
import { ProjectToolLoadError, ProjectToolRunError } from "./errors.ts";
import { validationMessage } from "./schema.ts";

export interface ProjectToolContext {
  readonly toolCallId: string;
  readonly projectRoot: string;
  readonly signal: AbortSignal | undefined;
}
type ProjectToolEffect = Effect.Effect<unknown, unknown, unknown>;
type ProjectToolExecute = (arguments_: unknown, context: ProjectToolContext) => ProjectToolEffect;

export interface ProjectToolModule {
  readonly execute: ProjectToolExecute;
  readonly dependencies?: Layer.Layer<unknown, unknown, unknown>;
}

export interface LoadedProjectTool extends LoadedManifest {
  readonly module: ProjectToolModule;
}

const effectEntry = fileURLToPath(import.meta.resolve("effect"));
const jiti = createJiti(import.meta.url, {
  alias: { effect: effectEntry },
  moduleCache: false,
});

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

export const loadProjectToolModule = (loaded: LoadedManifest) =>
  Effect.tryPromise({
    try: async () => {
      const imported = await jiti.import<unknown>(loaded.entrypoint);
      if (!isRecord(imported) || typeof imported.execute !== "function") {
        throw new ProjectToolLoadError({ path: loaded.entrypoint, message: "must export an execute function" });
      }
      if (imported.dependencies !== undefined && !Layer.isLayer(imported.dependencies)) {
        throw new ProjectToolLoadError({ path: loaded.entrypoint, message: "dependencies must be an Effect Layer" });
      }
      return {
        ...loaded,
        module: {
          execute: imported.execute as ProjectToolExecute,
          ...(imported.dependencies === undefined ? {} : { dependencies: imported.dependencies }),
        },
      } satisfies LoadedProjectTool;
    },
    catch: (cause) => cause instanceof ProjectToolLoadError
      ? cause
      : new ProjectToolLoadError({ path: loaded.entrypoint, message: "could not import project tool", cause }),
  });

function formatFailure(cause: Cause.Cause<unknown>): string {
  const typed = Cause.findErrorOption(cause);
  if (Option.isSome(typed)) {
    const error = typed.value;
    if (typeof error === "string") return error;
    if (error instanceof Error) return error.message;
    try {
      const json = JSON.stringify(error);
      if (json !== undefined) return json;
    } catch {
      // Fall through to Effect's cause formatter.
    }
  }
  return Cause.pretty(cause);
}

export const executeProjectTool = (
  tool: LoadedProjectTool,
  arguments_: unknown,
  context: ProjectToolContext,
) =>
  Effect.gen(function*() {
    if (!tool.parametersValidator.Check(arguments_)) {
      return yield* new ProjectToolRunError({
        toolName: tool.manifest.name,
        message: `invalid arguments: ${validationMessage(tool.parametersValidator, arguments_)}`,
      });
    }

    const effect = yield* Effect.try({
      try: () => tool.module.execute(arguments_, Object.freeze({ ...context })),
      catch: (cause) => new ProjectToolRunError({
        toolName: tool.manifest.name,
        message: "execute threw before returning an Effect",
        cause,
      }),
    });
    if (!Effect.isEffect(effect)) {
      return yield* new ProjectToolRunError({
        toolName: tool.manifest.name,
        message: "execute must return an Effect",
      });
    }

    const runnable = tool.module.dependencies === undefined
      ? effect
      : Effect.provide(effect, tool.module.dependencies);
    const exit = yield* Effect.promise(() => Effect.runPromiseExit(runnable as Effect.Effect<unknown, unknown>, {
      signal: context.signal,
    }));
    if (Exit.isFailure(exit)) {
      return yield* new ProjectToolRunError({
        toolName: tool.manifest.name,
        message: context.signal?.aborted ? "execution cancelled" : formatFailure(exit.cause),
        cause: exit.cause,
      });
    }
    if (!tool.resultValidator.Check(exit.value)) {
      return yield* new ProjectToolRunError({
        toolName: tool.manifest.name,
        message: `invalid result: ${validationMessage(tool.resultValidator, exit.value)}`,
      });
    }
    return exit.value;
  });

export function formatProjectToolResult(value: unknown): string {
  if (typeof value === "string") return value;
  return JSON.stringify(value, null, 2);
}
