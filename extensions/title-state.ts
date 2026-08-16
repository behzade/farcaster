import { basename } from "node:path";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Effect, type Fiber } from "effect";

const frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

export default function (pi: ExtensionAPI) {
  let animation: Fiber.Fiber<never, never> | undefined;
  let frame = 0;
  let activeContext: ExtensionContext | undefined;
  let awaitingApproval = false;

  const suffix = (ctx: ExtensionContext) => {
    const cwd = basename(ctx.cwd) || ctx.cwd;
    const session = pi.getSessionName();
    return session ? `π · ${session} · ${cwd}` : `π · ${cwd}`;
  };

  const stop = () => {
    animation?.interruptUnsafe();
    animation = undefined;
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
    animation = Effect.runFork(Effect.forever(
      Effect.sync(() => {
        ctx.ui.setTitle(`${frames[frame++ % frames.length]} ${suffix(ctx)}`);
      }).pipe(Effect.andThen(Effect.sleep(80))),
    ));
  };

  const unsubscribeRequested = pi.events.on("approval:requested", () => {
    if (!activeContext) return;
    awaitingApproval = true;
    stop();
    activeContext.ui.setTitle(`? ${suffix(activeContext)} · input needed`);
  });

  const unsubscribeResolved = pi.events.on("approval:resolved", () => {
    awaitingApproval = false;
    if (activeContext) setWorking(activeContext);
  });

  pi.on("session_start", (_event, ctx) => {
    awaitingApproval = false;
    setIdle(ctx);
  });
  pi.on("session_info_changed", (_event, ctx) => {
    activeContext = ctx;
    if (awaitingApproval) ctx.ui.setTitle(`? ${suffix(ctx)} · input needed`);
    else if (animation) setWorking(ctx);
    else setIdle(ctx);
  });
  pi.on("agent_start", (_event, ctx) => {
    awaitingApproval = false;
    setWorking(ctx);
  });
  pi.on("agent_settled", (_event, ctx) => {
    if (!awaitingApproval && !ctx.hasPendingMessages()) setIdle(ctx);
  });
  pi.on("session_shutdown", () => {
    stop();
    activeContext = undefined;
    awaitingApproval = false;
    unsubscribeRequested();
    unsubscribeResolved();
  });
}
