import {
  type AppKeybinding,
  type Keybinding,
  type KeyId,
  KeybindingsManager,
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

export const makeKeybindings = (
  manager: KeybindingsManager,
): KeybindingsShape => ({
  matches: (input, action) => manager.matches(input, action),
  keys: (action) => manager.getKeys(action),
  reload: () => manager.reload(),
})

export const KeybindingsLive = Layer.sync(Keybindings, () =>
  makeKeybindings(KeybindingsManager.create()),
)

export const primaryKey = (
  keybindings: KeybindingsShape,
  action: AppKeybinding,
): string => keybindings.keys(action)[0] ?? "unbound"
