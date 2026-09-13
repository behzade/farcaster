const marker = "farcaster-steering-resume";
export default function steering(pi) {
  let applying = false;
  let resumeStarted;
  pi.on("agent_start", () => {
    resumeStarted?.();
    resumeStarted = undefined;
  });
  pi.on("context", (event) => ({
    messages: event.messages.filter(message => message.customType !== marker),
  }));
  pi.registerCommand("farcaster-apply-steering", {
    description: "Apply pending Farcaster input",
    handler: async (_args, ctx) => {
      if (applying || (ctx.isIdle() && !ctx.hasPendingMessages())) return;
      applying = true;
      try {
        ctx.abort();
        await ctx.waitForIdle();
        // Start a new run without duplicating input already consumed before abort.
        // The context hook removes this hidden trigger before the provider sees it.
        await new Promise(resolve => {
          resumeStarted = resolve;
          pi.sendMessage({customType: marker, content: [], display: false}, {triggerTurn: true});
        });
      } finally {
        applying = false;
      }
    },
  });
}
