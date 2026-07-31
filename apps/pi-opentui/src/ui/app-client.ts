import type {
  AppCommand,
  AppSnapshot,
} from "../services/app-state.ts"
import type { ProjectPath } from "../services/project-paths.ts"

/**
 * The client boundary shared by terminal, web, and desktop views.
 * Keep renderer and input event types out of this interface.
 */
export interface AppClient {
  readonly initial: AppSnapshot
  readonly projectPaths: () => ReadonlyArray<ProjectPath>
  readonly subscribe: (
    listener: (snapshot: AppSnapshot) => void,
  ) => () => void
  readonly dispatch: (command: AppCommand) => void
  readonly quit: () => void
}
