// installed by wherewasi (https://github.com/Trolzie/herdr-wherewasi)
// managed by wherewasi; `wherewasi setup` overwrites this file.
// WHEREWASI_INTEGRATION_VERSION=__VERSION__
// @ts-nocheck

const BIN = "__WHEREWASI_BIN__";
const FOLLOW_UP_MARKER = "[wherewasi]";
const session = `pi-${process.pid}-${Date.now()}`;

export default function (pi) {
  // Only inside herdr, where the sidebar shows the file.
  if (process.env.HERDR_ENV !== "1") return;

  let nudged = false;

  const run = async (event, cwd) => {
    try {
      const result = await pi.exec(BIN, ["hook", "pi", event, session, cwd], { cwd, timeout: 10000 });
      return result.code === 0 ? result.stdout.trim() : "";
    } catch {
      return "";
    }
  };

  // Brief the agent as a hidden session message, and again only when the
  // file changed, so the system prompt (and pi's prompt cache) stays stable.
  let lastBriefing = "";
  pi.on("before_agent_start", async (event, ctx) => {
    if (!event.prompt.startsWith(FOLLOW_UP_MARKER)) {
      nudged = false;
      await run("baseline", ctx.cwd);
    }
    const briefing = await run("briefing", ctx.cwd);
    if (!briefing || briefing === lastBriefing) return;
    lastBriefing = briefing;
    return { message: { customType: "wherewasi", content: briefing, display: false } };
  });

  // Ask once for a log entry when the run changed the repo without one.
  pi.on("agent_end", async (event, ctx) => {
    if (nudged || ctx.hasPendingMessages()) return;
    const last = event.messages[event.messages.length - 1];
    if (last?.role === "assistant" && ["aborted", "error"].includes(last.stopReason)) return;
    const reason = await run("nudge", ctx.cwd);
    if (!reason) return;
    nudged = true;
    pi.sendUserMessage(`${FOLLOW_UP_MARKER} ${reason}`, { deliverAs: "followUp" });
  });
}
