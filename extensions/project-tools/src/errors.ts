import { Schema } from "effect";

export class ProjectToolLoadError extends Schema.TaggedError<ProjectToolLoadError>()(
  "ProjectToolLoadError",
  {
    path: Schema.String,
    message: Schema.String,
    cause: Schema.optional(Schema.Defect()),
  },
) {}

export class ProjectToolRunError extends Schema.TaggedError<ProjectToolRunError>()(
  "ProjectToolRunError",
  {
    toolName: Schema.String,
    message: Schema.String,
    cause: Schema.optional(Schema.Defect()),
  },
) {}
