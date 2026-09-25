// An editor with two windows open on the same SWEM.
//
// ACP lets a client hold as many sessions as it likes on one connection, and
// an editor is one process with several windows in it. This is the same SDK
// client as `editor.mjs`, opening two sessions over one connection and
// prompting each, so a walk can prove the door keeps them apart rather than
// keeping only the last one.
//
// Usage: two-windows.mjs <swem binary> <profile> <cwd the editor claims>
//        <what to say in the first> <what to say in the second>
// Prints one JSON line: a session id and what came back, per window.

import { spawn } from "node:child_process";
import { Readable, Writable } from "node:stream";
import { client, ndJsonStream } from "@agentclientprotocol/sdk";

const [binary, profile, claimedCwd, first, second] = process.argv.slice(2);
if (!binary || !profile || !claimedCwd || !first || !second) {
  console.error("usage: two-windows.mjs <swem> <profile> <cwd> <first> <second>");
  process.exit(2);
}

const swem = spawn(binary, ["acp", "--profile", profile], {
  stdio: ["pipe", "pipe", "pipe"],
  env: process.env,
});
const complaints = [];
swem.stderr.on("data", (chunk) => complaints.push(String(chunk)));
swem.on("error", (error) => {
  console.error(`could not start ${binary}: ${error.message}`);
  process.exit(3);
});

const stream = ndJsonStream(Writable.toWeb(swem.stdin), Readable.toWeb(swem.stdout));
// Every update, tagged with the session it arrived for: which is the whole
// question here.
const updates = [];
const timeout = setTimeout(() => {
  console.error(`the editor gave up waiting. stderr:\n${complaints.join("")}`);
  swem.kill("SIGKILL");
  process.exit(4);
}, 180_000);

const textOf = (sessionId) =>
  updates
    .filter((seen) => seen.session === sessionId && seen.update?.sessionUpdate === "agent_message_chunk")
    .map((seen) => (seen.update?.content?.type === "text" ? seen.update.content.text : ""))
    .join("");

try {
  const seen = await client({ name: "the-gates-editor", version: "0.1.0" })
    .onNotification("session/update", async ({ params }) => {
      updates.push({ session: params?.sessionId, update: params?.update });
    })
    .onRequest("session/request_permission", async ({ params }) => {
      const options = params?.options ?? [];
      const chosen = options.find((option) => option.kind === "allow_once") ?? options[0];
      return { outcome: { outcome: "selected", optionId: chosen?.optionId } };
    })
    .connectWith(stream, async (ctx) => {
      await ctx.request("initialize", {
        protocolVersion: 1,
        clientCapabilities: {},
        clientInfo: { name: "the-gates-editor", version: "0.1.0" },
      });
      // Both windows, before either is used: an editor opens its sessions as
      // its windows open, not one turn at a time.
      const windows = [];
      for (const said of [first, second]) {
        const session = await ctx.request("session/new", { cwd: claimedCwd, mcpServers: [] });
        windows.push({ session: session.sessionId, said });
      }
      for (const window of windows) {
        const answered = await ctx.request("session/prompt", {
          sessionId: window.session,
          prompt: [{ type: "text", text: window.said }],
        });
        window.stop = answered.stopReason;
      }
      // A person closes one window and keeps working in the other. ACP says
      // so with `session/close`; what must not happen is the other window
      // going with it.
      const closed = { session: windows[0].session };
      try {
        await ctx.request("session/close", { sessionId: windows[0].session });
        closed.ok = true;
      } catch (error) {
        closed.ok = false;
        closed.why = error?.message ?? String(error);
      }
      // The closed one is closed: a turn in it is refused rather than run.
      try {
        await ctx.request("session/prompt", {
          sessionId: windows[0].session,
          prompt: [{ type: "text", text: "anyone there?" }],
        });
        closed.stillAnswers = true;
      } catch {
        closed.stillAnswers = false;
      }
      // And the window that is still open still works.
      const after = await ctx.request("session/prompt", {
        sessionId: windows[1].session,
        prompt: [{ type: "text", text: windows[1].said }],
      });
      return { windows, closed, after: after.stopReason };
    });

  clearTimeout(timeout);
  console.log(
    JSON.stringify({
      windows: seen.windows.map((window) => ({
        session: window.session,
        stop: window.stop,
        text: textOf(window.session),
      })),
      closed: seen.closed,
      after: seen.after,
    }),
  );
  swem.kill("SIGTERM");
  process.exit(0);
} catch (error) {
  clearTimeout(timeout);
  console.error(`the editor could not talk to SWEM: ${error?.message ?? error}`);
  console.error(complaints.join(""));
  swem.kill("SIGKILL");
  process.exit(5);
}
