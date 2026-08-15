import { randomUUID } from "node:crypto";
import { join } from "node:path";
import { StringEnum } from "@earendil-works/pi-ai";
import {
  getAgentDir,
  type ExtensionAPI,
} from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import {
  appendAgentFeedback,
  FEEDBACK_CATEGORIES,
  FEEDBACK_SEVERITIES,
  type AgentFeedbackRecord,
} from "./lib/agent-feedback.ts";

const Params = Type.Object({
  category: StringEnum(FEEDBACK_CATEGORIES, {
    description: "Kind of Pi harness or setup issue",
  }),
  severity: StringEnum(FEEDBACK_SEVERITIES, {
    description: "Whether the issue blocks work, degrades it, or is minor friction",
  }),
  summary: Type.String({
    description: "Concise description of the concrete issue",
    minLength: 1,
    maxLength: 240,
  }),
  details: Type.String({
    description: "What happened, what was expected, and decisive evidence. Do not include secrets.",
    minLength: 1,
    maxLength: 4000,
  }),
  workaround: Type.Optional(Type.String({
    description: "Workaround used or the reason none is available",
    maxLength: 1000,
  })),
});

const feedbackPath = () => join(getAgentDir(), "agent-feedback.jsonl");

function notifyFromHeadlessChild(pi: ExtensionAPI, message: string): void {
  if (!process.env.PI_SUBAGENT_CHILD) return;
  if (process.platform === "darwin") {
    void pi.exec("terminal-notifier", [
      "-title",
      "Pi agent feedback",
      "-message",
      message,
      "-group",
      "pi-feedback",
      "-activate",
      "com.mitchellh.ghostty",
    ], { timeout: 5000 }).catch(() => undefined);
  } else if (process.platform === "linux") {
    void pi.exec("notify-send", ["--app-name=Pi", "Pi agent feedback", message], {
      timeout: 5000,
    }).catch(() => undefined);
  }
}

export default function (pi: ExtensionAPI) {
  pi.registerTool({
    name: "report_pi_feedback",
    label: "Report Pi feedback",
    description:
      "Record a concrete usability problem, irrational limitation, sandbox issue, setup failure, or bug in the Pi agent environment. Appends a private global JSONL log and notifies the user without waiting for a response. Use once per distinct Pi issue, not for ordinary project bugs. Continue the task when a safe workaround exists.",
    promptSnippet:
      "Report concrete Pi harness, sandbox, or setup friction without blocking on the user",
    promptGuidelines: [
      "Use report_pi_feedback once when a concrete Pi harness, sandbox, tool, or setup issue blocks or materially degrades work; do not use it for ordinary project defects.",
      "After report_pi_feedback, continue with a safe workaround when possible and never wait for a reply from the tool.",
      "Keep report_pi_feedback evidence specific and exclude credentials, secrets, and unnecessary user data.",
    ],
    parameters: Params,
    executionMode: "sequential",
    async execute(toolCallId, params, _signal, _onUpdate, ctx) {
      const path = feedbackPath();
      const sessionId = ctx.sessionManager.getSessionId();
      const sessionFile = ctx.sessionManager.getSessionFile();
      const runId = process.env.PI_SUBAGENT_RUN_ID;
      const model = ctx.model ? `${ctx.model.provider}/${ctx.model.id}` : undefined;
      const record: AgentFeedbackRecord = {
        version: 1,
        id: randomUUID(),
        timestamp: new Date().toISOString(),
        category: params.category,
        severity: params.severity,
        summary: params.summary.trim(),
        details: params.details.trim(),
        ...(params.workaround?.trim() ? { workaround: params.workaround.trim() } : {}),
        cwd: ctx.cwd,
        ...(sessionId ? { sessionId } : {}),
        ...(sessionFile ? { sessionFile } : {}),
        agent: process.env.PI_SUBAGENT_CHILD_AGENT ?? "main",
        ...(runId ? { runId } : {}),
        toolCallId,
        ...(model ? { model } : {}),
      };

      appendAgentFeedback(path, record);
      const notice = `${record.severity}: ${record.summary}`;
      ctx.ui.notify(`Pi feedback recorded — ${notice}`, "warning");
      if (process.env.PI_SUBAGENT_CHILD) {
        notifyFromHeadlessChild(pi, notice);
      } else {
        pi.events.emit("agent-feedback:reported", {
          title: "Pi agent feedback",
          message: notice,
          record,
        });
      }

      return {
        content: [{
          type: "text",
          text: `Feedback recorded in ${path}. Continue with a safe workaround if possible; no user response is required.`,
        }],
        details: { id: record.id, path },
      };
    },
  });
}
