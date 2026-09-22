export default {
  id: "farcaster.prompt-boundary",
  async setup(ctx) {
    const url = process.env.FARCASTER_PROMPT_BOUNDARY_URL;
    if (!url) return;
    await ctx.tool.hook("execute.after", async (event) => {
      const response = await fetch(url, {
        method: "POST",
        headers: {"Content-Type": "application/json"},
        body: JSON.stringify({session_id: event.sessionID}),
      });
      if (!response.ok) throw new Error("Farcaster prompt boundary unavailable");
      await response.json();
    });
  },
};
