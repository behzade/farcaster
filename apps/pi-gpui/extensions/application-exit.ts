import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export const APPLICATION_EXIT_MESSAGE =
  "Work was interrupted because the application exited.";

export default function applicationExit(pi: ExtensionAPI): void {
  pi.on("session_shutdown", (event, ctx) => {
    if (event.reason !== "quit" || ctx.isIdle()) return;
    ctx.sessionManager.appendCustomMessageEntry(
      "pi-gpui-application-exit",
      APPLICATION_EXIT_MESSAGE,
      true,
      { reason: "application-exit" },
    );
  });
}
