import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Effect } from "effect";
import { basename } from "node:path";
import { discoverProjectTools } from "./discovery.ts";
import { ProjectToolRunError } from "./errors.ts";
import { executeProjectTool, formatProjectToolResult, type LoadedProjectTool } from "./module.ts";

function projectSlug(projectRoot: string): string {
  const slug = basename(projectRoot).toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "");
  return slug || "project";
}

export function registeredName(projectRoot: string, toolName: string): string {
  return `project_${projectSlug(projectRoot)}_${toolName}`;
}

function registerTool(pi: ExtensionAPI, projectRoot: string, tool: LoadedProjectTool): string {
  const name = registeredName(projectRoot, tool.manifest.name);
  pi.registerTool({
    name,
    label: tool.manifest.label,
    description: tool.manifest.description,
    parameters: tool.manifest.parameters,
    async execute(toolCallId, arguments_, signal) {
      try {
        const value = await Effect.runPromise(executeProjectTool(tool, arguments_, {
          toolCallId,
          projectRoot,
          signal,
        }));
        return {
          content: [{ type: "text", text: formatProjectToolResult(value) }],
          details: { projectTool: tool.manifest.name },
        };
      } catch (cause) {
        if (cause instanceof ProjectToolRunError) throw new Error(cause.message, { cause });
        throw cause;
      }
    },
  });
  return name;
}

async function loadForSession(pi: ExtensionAPI, ctx: ExtensionContext): Promise<void> {
  if (!ctx.isProjectTrusted()) return;
  const discovery = await Effect.runPromise(discoverProjectTools(ctx.cwd));
  const diagnostics = [...discovery.diagnostics];
  const occupied = new Set(pi.getAllTools().map((tool) => tool.name));
  const registered: string[] = [];
  for (const tool of discovery.tools) {
    const name = registeredName(discovery.projectRoot, tool.manifest.name);
    if (occupied.has(name)) {
      diagnostics.push({ tool: tool.manifest.name, message: `tool name collides with ${name}` });
      continue;
    }
    registered.push(registerTool(pi, discovery.projectRoot, tool));
    occupied.add(name);
  }
  if (registered.length > 0) {
    pi.setActiveTools([...new Set([...pi.getActiveTools(), ...registered])]);
  }
  if (ctx.hasUI) {
    for (const diagnostic of diagnostics) {
      ctx.ui.notify(`Project tool ${diagnostic.tool} disabled: ${diagnostic.message}`, "warning");
    }
  }
}

export default function projectTools(pi: ExtensionAPI): void {
  pi.on("session_start", async (event, ctx) => {
    if (event.reason !== "startup" && event.reason !== "reload") return;
    await loadForSession(pi, ctx);
  });
}
