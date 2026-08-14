import { expect, test } from "bun:test"
import { buildSystemPrompt } from "../node_modules/@earendil-works/pi-coding-agent/dist/core/system-prompt.js"

const marker = "<!-- pi:active-tool-guidance -->"

const options = {
  customPrompt: `Custom persona.\n\n${marker}`,
  selectedTools: ["read", "bash"],
  toolSnippets: {
    read: "Read files",
    bash: "Run commands",
    inactive: "Must stay hidden",
  },
  promptGuidelines: ["Use read before edit.", "Use read before edit."],
  appendSystemPrompt: "APPEND CONTRACT",
  cwd: "/work/project",
  contextFiles: [{ path: "/work/project/AGENTS.md", content: "PROJECT CONTRACT" }],
  skills: [{
    name: "review",
    description: "Review changes",
    filePath: "/skills/review/SKILL.md",
    baseDir: "/skills/review",
    sourceInfo: { path: "/skills/review/SKILL.md", source: "test", scope: "temporary", origin: "top-level" },
    disableModelInvocation: false,
  }],
}

test("marked custom prompts append active tool guidance after stable context", () => {
  const prompt = buildSystemPrompt(options)

  expect(prompt).not.toContain(marker)
  expect(prompt).toContain("- read: Read files")
  expect(prompt).toContain("- bash: Run commands")
  expect(prompt).not.toContain("Must stay hidden")
  expect(prompt.match(/Use read before edit\./g)).toHaveLength(1)
  expect(prompt).toContain("Use bash for file operations like ls, rg, find")

  const append = prompt.indexOf("APPEND CONTRACT")
  const project = prompt.indexOf("PROJECT CONTRACT")
  const skill = prompt.indexOf("<name>review</name>")
  const cwd = prompt.indexOf("Current working directory: /work/project")
  const guidance = prompt.indexOf("Available tools:")
  expect(append).toBeGreaterThan(-1)
  expect(project).toBeGreaterThan(append)
  expect(skill).toBeGreaterThan(project)
  expect(cwd).toBeGreaterThan(skill)
  expect(guidance).toBeGreaterThan(cwd)
})

test("marker-free custom prompts retain complete-replacement semantics", () => {
  const prompt = buildSystemPrompt({
    ...options,
    customPrompt: "Exact custom prompt.",
  })

  expect(prompt).not.toContain("Available tools:")
  expect(prompt).not.toContain("Use read before edit.")
  expect(prompt).toStartWith("Exact custom prompt.\n\nAPPEND CONTRACT")
  expect(prompt).toEndWith("Current working directory: /work/project\n")
})

test("default prompts retain standard active guidance", () => {
  const prompt = buildSystemPrompt({
    ...options,
    customPrompt: undefined,
    appendSystemPrompt: undefined,
    contextFiles: [],
    skills: [],
    promptGuidelines: ["Use read before edit.", " Use read before edit. "],
  })

  expect(prompt).toContain("- read: Read files")
  expect(prompt).not.toContain("Must stay hidden")
  expect(prompt.match(/Use read before edit\./g)).toHaveLength(1)
  expect(prompt).toContain("- Be concise in your responses")
  expect(prompt).toContain("- Show file paths clearly when working with files")
})

test("active snippets retain the caller's tool order", () => {
  const prompt = buildSystemPrompt({
    ...options,
    selectedTools: ["bash", "read"],
  })

  expect(prompt.indexOf("- bash: Run commands")).toBeLessThan(prompt.indexOf("- read: Read files"))
})

test("skills stay hidden when read is inactive", () => {
  const prompt = buildSystemPrompt({
    ...options,
    selectedTools: ["bash"],
  })

  expect(prompt).not.toContain("<name>review</name>")
  expect(prompt).toContain("- bash: Run commands")
})

test("marked custom prompts reject duplicate markers", () => {
  expect(() => buildSystemPrompt({
    ...options,
    customPrompt: `${marker}\n${marker}`,
  })).toThrow("must contain at most one")
})

test("marked prompt assembly is byte-deterministic", () => {
  const first = buildSystemPrompt(options)
  const second = buildSystemPrompt({
    ...options,
    selectedTools: [...options.selectedTools],
    toolSnippets: { ...options.toolSnippets },
    promptGuidelines: [...options.promptGuidelines],
    contextFiles: options.contextFiles.map((file) => ({ ...file })),
    skills: options.skills.map((skill) => ({ ...skill })),
  })

  expect(second).toBe(first)
})
