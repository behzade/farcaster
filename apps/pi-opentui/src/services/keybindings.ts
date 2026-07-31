import {
  type AppKeybinding,
  type Keybinding,
  type KeyId,
  KeybindingsManager,
  setKittyProtocolActive,
} from "@earendil-works/pi-coding-agent"
import { Context, Layer } from "effect"

export interface KeybindingsShape {
  readonly matches: (input: string, action: Keybinding) => boolean
  readonly keys: (action: Keybinding) => ReadonlyArray<KeyId>
  readonly reload: () => void
}

export class Keybindings extends Context.Tag("pi-opentui/Keybindings")<
  Keybindings,
  KeybindingsShape
>() {}

const kittyNavigationKeys: Readonly<
  Record<number, readonly [number, string]>
> = {
  57348: [2, "~"],
  57349: [3, "~"],
  57350: [1, "D"],
  57351: [1, "C"],
  57352: [1, "A"],
  57353: [1, "B"],
  57354: [5, "~"],
  57355: [6, "~"],
  57356: [1, "H"],
  57357: [1, "F"],
}

const normalizeOpenTuiKeyInput = (input: string): string => {
  const match = input.match(
    /^\u001b\[(\d+)(?:;(\d+))?(?::(\d+))?u$/,
  )
  if (match === null) return input
  const mapped = kittyNavigationKeys[Number(match[1])]
  if (mapped === undefined) return input
  const [number, suffix] = mapped
  const eventType = match[3] === undefined ? "" : `:${match[3]}`
  return `\u001b[${number};${match[2] ?? "1"}${eventType}${suffix}`
}

export const makeKeybindings = (
  manager: KeybindingsManager,
): KeybindingsShape => {
  setKittyProtocolActive(true)
  return {
    matches: (input, action) =>
      manager.matches(normalizeOpenTuiKeyInput(input), action),
    keys: (action) => manager.getKeys(action),
    reload: () => manager.reload(),
  }
}

export const KeybindingsLive = Layer.sync(Keybindings, () =>
  makeKeybindings(KeybindingsManager.create()),
)

export const primaryKey = (
  keybindings: KeybindingsShape,
  action: AppKeybinding,
): string => keybindings.keys(action)[0] ?? "unbound"
