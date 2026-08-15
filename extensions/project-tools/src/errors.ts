import { Data } from "effect";

export class ProjectToolLoadError extends Data.TaggedError("ProjectToolLoadError")<{
  readonly path: string;
  readonly message: string;
  readonly cause?: unknown;
}> {}
export class ProjectToolRunError extends Data.TaggedError("ProjectToolRunError")<{
  readonly toolName: string;
  readonly message: string;
  readonly cause?: unknown;
}> {}
