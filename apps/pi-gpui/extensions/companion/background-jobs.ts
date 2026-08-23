import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

export const BACKGROUND_JOBS_STATUS_KEY = "\u001fpi-gpui-background-jobs\u001f";

type JobState = "starting" | "running" | "completed" | "exited" | "failed";

interface BackgroundJob {
  name: string;
  command: string;
  state: JobState;
  exitCode?: number;
}

interface PendingCall {
  action: string;
  name?: string;
}

interface Settlement {
  name?: unknown;
  state?: unknown;
  exitCode?: unknown;
}

function bounded(value: string, length: number): string {
  return value.length <= length ? value : `${value.slice(0, length - 1)}…`;
}

function resultText(result: { content?: Array<{ type?: string; text?: string }> }): string {
  return result.content
    ?.filter((part) => part.type === "text" && typeof part.text === "string")
    .map((part) => part.text)
    .join("\n") ?? "";
}

export default function backgroundJobs(pi: ExtensionAPI): void {
  const jobs = new Map<string, BackgroundJob>();
  const pendingCalls = new Map<string, PendingCall>();

  const publish = (ctx: ExtensionContext): void => {
    ctx.ui.setStatus(BACKGROUND_JOBS_STATUS_KEY, JSON.stringify([...jobs.values()]));
  };

  pi.on("session_start", (_event, ctx) => {
    jobs.clear();
    pendingCalls.clear();
    publish(ctx);
  });

  pi.on("tool_execution_start", (event, ctx) => {
    if (event.toolName !== "background_job" || typeof event.args?.action !== "string") return;
    const name = typeof event.args.name === "string" ? event.args.name : undefined;
    pendingCalls.set(event.toolCallId, { action: event.args.action, name });
    if (event.args.action !== "start" || !name || typeof event.args.command !== "string") return;
    jobs.set(name, {
      name,
      command: bounded(event.args.command, 240),
      state: "starting",
    });
    publish(ctx);
  });

  pi.on("tool_execution_end", (event, ctx) => {
    if (event.toolName !== "background_job") return;
    const call = pendingCalls.get(event.toolCallId);
    pendingCalls.delete(event.toolCallId);
    if (!call?.name) return;
    const output = resultText(event.result);
    if (call.action === "start") {
      const job = jobs.get(call.name);
      if (!job) return;
      if (event.isError || !output.startsWith(`started ${call.name}`)) jobs.delete(call.name);
      else if (job.state === "starting") job.state = "running";
      publish(ctx);
    } else if (call.action === "stop" && output.startsWith(`stopped ${call.name}`)) {
      jobs.delete(call.name);
      publish(ctx);
    }
  });

  pi.on("message_end", (event, ctx) => {
    if (event.message.role !== "custom" || event.message.customType !== "background-job-result") return;
    const details = event.message.details as Settlement | undefined;
    if (
      typeof details?.name !== "string" ||
      !["completed", "exited", "failed"].includes(String(details.state))
    ) return;
    const job = jobs.get(details.name);
    if (!job) return;
    job.state = details.state as "completed" | "exited" | "failed";
    if (typeof details.exitCode === "number") job.exitCode = details.exitCode;
    publish(ctx);
  });
}
