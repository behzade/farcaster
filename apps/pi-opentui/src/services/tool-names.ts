export type CanonicalToolName = "read" | "edit" | "write" | "bash" | "generic"

const toolAliases: Readonly<Record<string, Exclude<CanonicalToolName, "generic">>> = {
  read: "read",
  read_file: "read",
  file_read: "read",
  read_tool: "read",
  edit: "edit",
  edit_file: "edit",
  file_edit: "edit",
  write: "write",
  write_file: "write",
  file_write: "write",
  bash: "bash",
  bash_tool: "bash",
  shell: "bash",
  shell_command: "bash",
  exec: "bash",
  execute: "bash",
  run_command: "bash",
  execute_command: "bash",
}

const normalizedToolName = (toolName: string): string =>
  toolName
    .replace(/([a-z\d])([A-Z])/g, "$1_$2")
    .trim()
    .toLocaleLowerCase()
    .replace(/[^a-z\d]+/g, "_")
    .replace(/^_+|_+$/g, "")

export const canonicalToolName = (toolName: string): CanonicalToolName =>
  toolAliases[normalizedToolName(toolName)] ?? "generic"

export const isReadToolName = (toolName: string): boolean =>
  canonicalToolName(toolName) === "read"
