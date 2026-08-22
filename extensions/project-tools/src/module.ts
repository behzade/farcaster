import { Cause, Effect, Exit, Layer, Option } from "effect";
import { createJiti } from "jiti";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import type { LoadedManifest } from "./manifest.ts";
import { ProjectToolLoadError, ProjectToolRunError } from "./errors.ts";
import { validationMessage } from "./schema.ts";
import { truncateProjectToolOutput, type BoundedProjectToolOutput } from "./truncation.ts";

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
  alias: { effect: dirname(effectEntry) },
  moduleCache: false,
});

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

export const loadProjectToolModule = Effect.fn("ProjectTools.loadProjectToolModule")(
  (loaded: LoadedManifest) => Effect.tryPromise({
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
  }),
);

const interruptOnAbort = (signal: AbortSignal): Effect.Effect<never> =>
  Effect.callback<never>((resume) => {
    if (signal.aborted) {
      resume(Effect.interrupt);
      return;
    }
    const onAbort = () => resume(Effect.interrupt);
    signal.addEventListener("abort", onAbort, { once: true });
    return Effect.sync(() => signal.removeEventListener("abort", onAbort));
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

export const executeProjectTool = Effect.fn("ProjectTools.executeProjectTool")(
  function* (
    tool: LoadedProjectTool,
    arguments_: unknown,
    context: ProjectToolContext,
  ) {
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

    const provided = tool.module.dependencies === undefined
      ? effect
      : Effect.provide(effect, tool.module.dependencies);
    const runnable = context.signal === undefined
      ? provided
      : Effect.raceFirst(provided, interruptOnAbort(context.signal));
    const exit = yield* Effect.exit(runnable as Effect.Effect<unknown, unknown>);
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
  },
);

export function formatProjectToolResult(value: unknown): BoundedProjectToolOutput {
  const output = typeof value === "string" ? value : JSON.stringify(value, null, 2);
  return truncateProjectToolOutput(output);
}
