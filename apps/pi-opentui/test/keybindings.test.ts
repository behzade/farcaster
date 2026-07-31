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

test("matches modified navigation input from terminals", () => {
  const keybindings = makeKeybindings(new KeybindingsManager())
  expect(
    keybindings.matches("\u001b[1;3A", "app.message.dequeue"),
  ).toBe(true)
  expect(
    keybindings.matches("\u001b[57352;3u", "app.message.dequeue"),
  ).toBe(true)
})
