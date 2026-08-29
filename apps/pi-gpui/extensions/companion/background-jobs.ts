import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

export const BACKGROUND_JOBS_STATUS_KEY = "\u001fpi-gpui-background-jobs\u001f";

const SETTLED_STATES = ["completed", "exited", "failed"] as const;
type SettledState = typeof SETTLED_STATES[number];

interface BackgroundJob {
  name: string;
  command: string;
  state: "running" | SettledState;
  exitCode?: number;
}

interface ProcessSettlement {
  id: string;
  state: SettledState;
  exitCode?: number;
}

function bounded(value: string, length: number): string {
  return value.length <= length ? value : `${value.slice(0, length - 1)}…`;
}

function record(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" ? value as Record<string, unknown> : undefined;
}

function runningProcessId(value: unknown): string | undefined {
  const details = record(value);
  return details?.state === "running" && typeof details.id === "string" ? details.id : undefined;
}

function isSettledState(value: unknown): value is SettledState {
  return typeof value === "string" && SETTLED_STATES.some((state) => state === value);
}

function processSettlement(value: unknown): ProcessSettlement | undefined {
  const details = record(value);
  if (typeof details?.id !== "string" || !isSettledState(details.state)) return undefined;
  return {
    id: details.id,
    state: details.state,
    ...(typeof details.exitCode === "number" ? { exitCode: details.exitCode } : {}),
  };
}

export default function backgroundJobs(pi: ExtensionAPI): void {
  const jobs = new Map<string, BackgroundJob>();
  const pendingBashCommands = new Map<string, string>();
  const pendingSettlements = new Map<string, ProcessSettlement>();

  const publish = (ctx: ExtensionContext): void => {
    ctx.ui.setStatus(BACKGROUND_JOBS_STATUS_KEY, JSON.stringify([...jobs.values()]));
  };
  const settle = (settlement: ProcessSettlement, ctx: ExtensionContext): void => {
    const job = jobs.get(settlement.id);
    if (!job) {
      pendingSettlements.set(settlement.id, settlement);
      return;
    }
    job.state = settlement.state;
    if (settlement.exitCode !== undefined) job.exitCode = settlement.exitCode;
    publish(ctx);
  };

  pi.on("session_start", (_event, ctx) => {
    jobs.clear();
    pendingBashCommands.clear();
    pendingSettlements.clear();
    publish(ctx);
  });

  pi.on("tool_execution_start", (event) => {
    if (event.toolName !== "bash" || typeof event.args?.command !== "string") return;
    pendingBashCommands.set(event.toolCallId, bounded(event.args.command, 240));
  });

  pi.on("tool_execution_end", (event, ctx) => {
    if (event.toolName !== "bash") return;
    const command = pendingBashCommands.get(event.toolCallId);
    pendingBashCommands.delete(event.toolCallId);
    const id = runningProcessId(event.result.details);
    if (event.isError || !command || !id) return;
    jobs.set(id, { name: id, command, state: "running" });
    const settlement = pendingSettlements.get(id);
    if (!settlement) {
      publish(ctx);
      return;
    }
    pendingSettlements.delete(id);
    settle(settlement, ctx);
  });

  pi.on("message_end", (event, ctx) => {
    if (event.message.role !== "custom" || event.message.customType !== "process-session-result") return;
    const settlement = processSettlement(event.message.details);
    if (settlement) settle(settlement, ctx);
  });
}
