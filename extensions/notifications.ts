import { existsSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { getAgentDir } from "@earendil-works/pi-coding-agent";
import { Effect, type Fiber } from "effect";
import { NotificationCoalescer, osc9Sequence, preview } from "./lib/notification-core.ts";
type Condition = "always" | "unfocused" | "never";
type NotificationType =
  | "agent-feedback"
  | "agent-turn-complete"
  | "approval-command"
  | "approval-file-write"
  | "user-input";

interface NotificationConfig {
  condition: Condition;
  types: NotificationType[];
  method: "auto" | "osc9" | "native" | "bel";
}

interface PendingNotification {
  type: NotificationType;
  title: string;
  message: string;
  priority: number;
}

const defaults: NotificationConfig = {
  condition: "unfocused",
  types: ["agent-feedback", "agent-turn-complete", "approval-command", "approval-file-write", "user-input"],
  method: "auto",
};

function loadConfig(): NotificationConfig {
  const path = join(getAgentDir(), "extensions", "notifications.json");
  if (!existsSync(path)) return defaults;
  try {
    const configured = JSON.parse(readFileSync(path, "utf8")) as Partial<NotificationConfig>;
    return {
      ...defaults,
      ...configured,
      types: [...new Set(["agent-feedback" as const, ...(configured.types ?? defaults.types)])],
    };
  } catch (error) {
    console.error(`Could not load notification settings ${path}: ${error}`);
    return defaults;
  }
}

function supportsOsc9(): boolean {
  const terminal = process.env.TERM_PROGRAM?.toLowerCase();
  return (
    terminal === "ghostty" || terminal === "iterm.app" || terminal === "wezterm" ||
    terminal === "warpterminal" || process.env.KITTY_WINDOW_ID !== undefined
  );
}

export default function (pi: ExtensionAPI) {
  const config = loadConfig();
  let postGpuiNotification: ((title: string, message: string) => void) | undefined;
  let lastAssistantText = "Turn ended";
  let terminalFocused = true;
  let focusTail = "";
  const queue = new NotificationCoalescer<PendingNotification>();
  let flushFiber: Fiber.Fiber<void, never> | undefined;
  let focusReporting = false;

  const shouldPost = (type: NotificationType) =>
    config.types.includes(type) && config.condition !== "never" &&
    (config.condition === "always" || !terminalFocused);

  const post = Effect.fn("Notifications.post")(function* ({ title, message }: PendingNotification) {
    if (postGpuiNotification) {
      yield* Effect.sync(() => postGpuiNotification?.(title, message));
    } else if ((config.method === "auto" && supportsOsc9()) || config.method === "osc9") {
      yield* Effect.sync(() => process.stdout.write(osc9Sequence(message, Boolean(process.env.TMUX))));
    } else if (config.method === "bel") {
      yield* Effect.sync(() => process.stdout.write("\u0007"));
    } else if (process.platform === "darwin") {
      yield* Effect.tryPromise({
        try: () => pi.exec("terminal-notifier", ["-title", title, "-message", preview(message), "-group", "pi", "-activate", "com.mitchellh.ghostty"]),
        catch: () => undefined,
      }).pipe(Effect.ignore);
    } else if (process.platform === "linux") {
      yield* Effect.tryPromise({
        try: () => pi.exec("notify-send", ["--app-name=Pi", title, preview(message)]),
        catch: () => undefined,
      }).pipe(Effect.ignore);
    }
  });

  const flush = Effect.fn("Notifications.flush")(function* () {
    while (true) {
      const next = yield* Effect.sync(() => queue.take());
      if (!next) return;
      if (shouldPost(next.type)) yield* post(next);
    }
  });

  const enqueue = (next: PendingNotification) => {
    if (!shouldPost(next.type)) return;
    queue.push(next);
    if (flushFiber) return;
    flushFiber = Effect.runFork(
      Effect.sleep(75).pipe(
        Effect.andThen(flush()),
        Effect.ensuring(Effect.sync(() => {
          flushFiber = undefined;
        })),
      ),
    );
  };

  const onInput = (chunk: Buffer | string) => {
    focusTail = `${focusTail}${chunk.toString()}`.slice(-32);
    const gained = focusTail.lastIndexOf("\u001b[I");
    const lost = focusTail.lastIndexOf("\u001b[O");
    if (gained >= 0 || lost >= 0) terminalFocused = gained > lost;
  };

  const unsubscribeApproval = pi.events.on("approval:requested", (data: unknown) => {
    const request = data as { kind?: string; title?: string; summary?: string };
    const type = request.kind === "command" ? "approval-command"
      : request.kind === "file-write" ? "approval-file-write"
      : "user-input";
    enqueue({ type, title: request.title ?? "Pi needs approval", message: request.summary ?? "Input needed", priority: type === "user-input" ? 3 : 2 });
  });
  const unsubscribeFeedback = pi.events.on("agent-feedback:reported", (data: unknown) => {
    const feedback = data as { title?: string; message?: string };
    enqueue({
      type: "agent-feedback",
      title: feedback.title ?? "Pi agent feedback",
      message: feedback.message ?? "New Pi feedback recorded",
      priority: 3,
    });
  });

  pi.on("session_start", (_event, ctx) => {
    terminalFocused = ctx.mode === "tui";
    if (process.env.PI_GPUI_NATIVE_NOTIFICATIONS === "1") {
      postGpuiNotification = (title, message) => {
        ctx.ui.notify(`\u001fpi-gpui-notification\u001f${title}\u001f${preview(message)}`, "info");
      };
    }
    if (ctx.mode === "tui" && process.stdin.isTTY) {
      process.stdin.on("data", onInput);
      process.stdout.write("\u001b[?1004h");
      focusReporting = true;
    }
  });

  pi.on("message_end", (event) => {
    if (event.message.role !== "assistant") return;
    const text = event.message.content.filter((part) => part.type === "text").map((part) => part.text).join("\n");
    if (text.trim()) lastAssistantText = preview(text);
  });

  pi.on("agent_settled", (_event, ctx) => {
    if (ctx.hasPendingMessages()) return;
    enqueue({ type: "agent-turn-complete", title: `Pi finished — ${basename(ctx.cwd) || ctx.cwd}`, message: lastAssistantText, priority: 1 });
  });

  pi.on("session_shutdown", () => {
    postGpuiNotification = undefined;
    unsubscribeApproval();
    unsubscribeFeedback();
    flushFiber?.interruptUnsafe();
    flushFiber = undefined;
    if (focusReporting) {
      process.stdin.off("data", onInput);
      process.stdout.write("\u001b[?1004l");
    }
  });

  pi.registerCommand("notifications", {
    description: "Show notification settings",
    handler: (_args, ctx) => ctx.ui.notify(`Notifications: ${config.condition}; ${config.method}; ${config.types.join(", ")}`, "info"),
  });
}
