import type {
  AppSnapshot,
  CommandInfo,
} from "../services/app-state.ts"

export interface HeaderViewModel {
  readonly activity: string
  readonly location: string
  readonly usage: string
}

const modelText = (snapshot: AppSnapshot): string => {
  const model = snapshot.model
  if (model === undefined) return "no model"
  const thinking = model.reasoning
    ? ` · thinking ${snapshot.thinkingLevel}`
    : ""
  return `${model.provider}/${model.id}${thinking}`
}

const usageText = (snapshot: AppSnapshot): string => {
  const stats = snapshot.sessionStats
  const context =
    stats.contextUsage?.percent == null
      ? ""
      : ` · ctx ${Math.round(stats.contextUsage.percent)}%`
  return `${snapshot.phase} · ${stats.tokens.total.toLocaleString()} tokens${context} · $${stats.cost.toFixed(4)}`
}

export const headerViewModel = (
  snapshot: AppSnapshot,
): HeaderViewModel => {
  const statuses = Object.values(snapshot.statuses)
  return {
    activity: `${snapshot.activeTools.length} tools · ${snapshot.extensionPaths.length} extensions`,
    location: `${snapshot.cwd} · ${modelText(snapshot)}${statuses.length > 0 ? ` · ${statuses.join(" · ")}` : ""}`,
    usage: usageText(snapshot),
  }
}

export const commandMenuOptions = (
  commands: ReadonlyArray<CommandInfo>,
): ReadonlyArray<string> =>
  commands.map(
    (command) =>
      `/${command.name}${command.description.length > 0 ? ` — ${command.description}` : ""}`,
  )

export const commandNameFromMenuOption = (
  selected: string | undefined,
): string | undefined => selected?.slice(1).split(/\s/, 1)[0]
