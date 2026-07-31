import { KeybindingsManager } from "@earendil-works/pi-coding-agent"
import { expect, test } from "bun:test"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { makeKeybindings } from "../src/services/keybindings.ts"

test("loads and reloads Pi keybinding overrides", async () => {
  const agentDir = await mkdtemp(join(tmpdir(), "pi-opentui-keys-"))
  try {
    await writeFile(
      join(agentDir, "keybindings.json"),
      JSON.stringify({ "app.exit": "ctrl+q" }),
    )
    const keybindings = makeKeybindings(
      KeybindingsManager.create(agentDir),
    )
    expect(keybindings.keys("app.exit")).toEqual(["ctrl+q"])

    await writeFile(
      join(agentDir, "keybindings.json"),
      JSON.stringify({ "app.exit": "ctrl+x" }),
    )
    keybindings.reload()
    expect(keybindings.keys("app.exit")).toEqual(["ctrl+x"])
  } finally {
    await rm(agentDir, { recursive: true, force: true })
  }
})
