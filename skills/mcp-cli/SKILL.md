---
name: mcp-cli
description: Access configured MCP servers through the stateless mcp-cli command. Use when discovering or invoking MCP server tools.
---

# MCP CLI

Access configured MCP servers through stateless `mcp-cli` commands in `bash`.

## Local servers

- Figma Desktop: `http://127.0.0.1:3845/mcp`

1. Discover servers and tools:
   ```sh
   mcp-cli grep <query>
   ```
2. Inspect a tool's input schema before calling it:
   ```sh
   mcp-cli info <server> <tool>
   ```
3. Invoke the tool with a JSON argument object:
   ```sh
   mcp-cli call <server> <tool> '<json>'
   ```

Do not assume a tool name or schema when it can be discovered with `grep` and `info`.
