import { basename } from "node:path";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

const frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

export default function (pi: ExtensionAPI) {
  let timer: ReturnType<typeof setInterval> | undefined;
  let frame = 0;
  let activeContext: ExtensionContext | undefined;

  const suffix = (ctx: ExtensionContext) => {
    const cwd = basename(ctx.cwd) || ctx.cwd;
    const session = pi.getSessionName();
    return session ? `π · ${session} · ${cwd}` : `π · ${cwd}`;
  };

  const stop = () => {
    if (timer) clearInterval(timer);
    timer = undefined;
    frame = 0;
  };

  const setIdle = (ctx: ExtensionContext) => {
    stop();
    activeContext = ctx;
    ctx.ui.setTitle(`● ${suffix(ctx)}`);
  };

  const setWorking = (ctx: ExtensionContext) => {
    stop();
    activeContext = ctx;
    const tick = () => {
      ctx.ui.setTitle(`${frames[frame++ % frames.length]} ${suffix(ctx)}`);
    };
    tick();
    timer = setInterval(tick, 80);
  };

  const unsubscribeRequested = pi.events.on("approval:requested", () => {
    if (!activeContext) return;
    stop();
    activeContext.ui.setTitle(`? ${suffix(activeContext)} · input needed`);
  });

  const unsubscribeResolved = pi.events.on("approval:resolved", () => {
    if (activeContext) setWorking(activeContext);
  });

  pi.on("session_start", (_event, ctx) => setIdle(ctx));
  pi.on("session_info_changed", (_event, ctx) => {
    if (timer) setWorking(ctx);
    else setIdle(ctx);
  });
  pi.on("agent_start", (_event, ctx) => setWorking(ctx));
  pi.on("agent_settled", (_event, ctx) => {
    if (!ctx.hasPendingMessages()) setIdle(ctx);
  });
  pi.on("session_shutdown", () => {
    stop();
    activeContext = undefined;
    unsubscribeRequested();
    unsubscribeResolved();
  });
}
