import {
  closeSync,
  constants,
  mkdirSync,
  openSync,
  writeSync,
} from "node:fs";
import { dirname } from "node:path";

export const FEEDBACK_CATEGORIES = [
  "usability",
  "limitation",
  "sandbox",
  "setup",
  "bug",
  "other",
] as const;

export const FEEDBACK_SEVERITIES = ["blocking", "degraded", "minor"] as const;

export type FeedbackCategory = (typeof FEEDBACK_CATEGORIES)[number];
export type FeedbackSeverity = (typeof FEEDBACK_SEVERITIES)[number];

export interface AgentFeedbackRecord {
  version: 1;
  id: string;
  timestamp: string;
  category: FeedbackCategory;
  severity: FeedbackSeverity;
  summary: string;
  details: string;
  workaround?: string;
  cwd: string;
  sessionId?: string;
  sessionFile?: string;
  agent: string;
  runId?: string;
  toolCallId: string;
  model?: string;
}

/** Append one bounded JSONL record without following a pre-existing symlink. */
export function appendAgentFeedback(path: string, record: AgentFeedbackRecord): void {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const fd = openSync(
    path,
    constants.O_APPEND |
      constants.O_CREAT |
      constants.O_WRONLY |
      constants.O_NOFOLLOW,
    0o600,
  );
  try {
    writeSync(fd, `${JSON.stringify(record)}\n`, undefined, "utf8");
  } finally {
    closeSync(fd);
  }
}
