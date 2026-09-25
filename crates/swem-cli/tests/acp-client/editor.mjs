// An editor, talking to SWEM over ACP.
//
// The client here is the ACP project's own TypeScript SDK, not something SWEM
// wrote: what it proves is that SWEM's editor door speaks the protocol as a
// third party implements it, rather than as our own fixture happens to expect.
// It is still not Zed, and the walk that runs this says so.
//
// Usage: editor.mjs <swem binary> <profile> <cwd the editor claims> <what to say>
//        [allow|reject] [session id to load] [what is in the editor's buffer]
// The fifth argument is what the person at the editor answers when the agent
// asks for permission; it defaults to allowing once. The sixth, when given, is
// a session id this editor kept from a previous launch: it loads that
// conversation instead of starting a new one, which is what an editor does
// when a person reopens a project.
//
// The seventh, when given at all, is what the person has on their screen. An
// editor with a file open holds a version of it the disk does not - the
// unsaved one - which is why ACP puts `fs/read_text_file` and
// `fs/write_text_file` on the client. Give it and this editor advertises both
// and answers them out of that buffer, the way a real one answers out of the
// window the person is typing in. Leave it off and the editor claims no
// filesystem, which is what it did before and what a browser tab does.
// Prints one JSON line: what the editor saw.

import { spawn } from "node:child_process";
import { Readable, Writable } from "node:stream";
import { client, ndJsonStream } from "@agentclientprotocol/sdk";

const [binary, profile, claimedCwd, said, answer = "allow", load = "", buffer] =
  process.argv.slice(2);
// Absent is not the same as empty: an editor may have an empty file open.
const hasBuffer = buffer !== undefined;
// What the person has on screen, and every file call that reached this editor.
let onScreen = buffer ?? "";
const files = [];
if (!binary || !profile || !claimedCwd || !said) {
  console.error("usage: editor.mjs <swem> <profile> <cwd> <text>");
  process.exit(2);
}

// This is how an editor starts an agent: as its own child, on stdio.
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

const stream = ndJsonStream(
  Writable.toWeb(swem.stdin),
  Readable.toWeb(swem.stdout),
);

const updates = [];
// What arrived before `session/load` answered: the conversation as the agent
// replayed it. An editor draws this as the history of the session it reopened.
let replayed = null;
// What the agent stopped to ask, and what the person at this editor said.
const asked = [];
const timeout = setTimeout(() => {
  console.error(`the editor gave up waiting. stderr:\n${complaints.join("")}`);
  swem.kill("SIGKILL");
  process.exit(4);
}, 120_000);

try {
  const seen = await client({ name: "the-gates-editor", version: "0.1.0" })
    // The SDK hands a handler its context; the notification itself is
    // `params`, shaped as ACP's SessionNotification.
    .onNotification("session/update", async ({ params }) => {
      updates.push(params?.update);
    })
    // The person is here, in their editor, so this is where the agent's
    // question belongs. An editor shows it and waits for a click; this one
    // answers the way the walk told it to.
    .onRequest("session/request_permission", async ({ params }) => {
      const options = params?.options ?? [];
      const wanted = answer === "reject" ? "reject_once" : "allow_once";
      const chosen =
        options.find((option) => option.kind === wanted) ?? options[0];
      asked.push({
        title: params?.toolCall?.title ?? null,
        options: options.map((option) => option.optionId),
        answered: chosen?.optionId ?? null,
      });
      return {outcome: {outcome: "selected", optionId: chosen?.optionId}};
    })
    // The file the person has open, handed over from the buffer rather than
    // read off the disk. An editor that answered from the disk would give the
    // agent a different file from the one on the screen.
    .onRequest("fs/read_text_file", async ({ params }) => {
      files.push({ method: "fs/read_text_file", path: params?.path ?? null });
      return { content: onScreen };
    })
    .onRequest("fs/write_text_file", async ({ params }) => {
      files.push({ method: "fs/write_text_file", path: params?.path ?? null });
      onScreen = params?.content ?? "";
      return {};
    })
    .connectWith(stream, async (ctx) => {
      const initialized = await ctx.request("initialize", {
        protocolVersion: 1,
        clientCapabilities: hasBuffer
          ? { fs: { readTextFile: true, writeTextFile: true } }
          : {},
        clientInfo: { name: "the-gates-editor", version: "0.1.0" },
      });
      // What an editor does when it has nothing kept: ask what conversations
      // there are, so a person can pick one instead of being expected to
      // remember an id.
      let listed = null;
      if (initialized.agentCapabilities?.sessionCapabilities?.list) {
        listed = (await ctx.request("session/list", {})).sessions ?? [];
      }
      // The editor names a directory, the way an editor does: the one the
      // person has open. SWEM is not supposed to take it.
      let session;
      if (load) {
        await ctx.request("session/load", {
          sessionId: load,
          cwd: claimedCwd,
          mcpServers: [],
        });
        // Everything the agent sent while answering the load is the
        // conversation it is handing back.
        replayed = updates.splice(0, updates.length);
        session = { sessionId: load };
      } else {
        session = await ctx.request("session/new", {
          cwd: claimedCwd,
          mcpServers: [],
        });
      }
      const answered = await ctx.request("session/prompt", {
        sessionId: session.sessionId,
        prompt: [{ type: "text", text: said }],
      });
      return { initialized, listed, session, answered };
    });

  clearTimeout(timeout);
  const text = updates
    .filter((update) => update?.sessionUpdate === "agent_message_chunk")
    .map((update) => (update?.content?.type === "text" ? update.content.text : ""))
    .join("");
  console.log(
    JSON.stringify({
      agent: seen.initialized.agentInfo?.name ?? null,
      protocol: seen.initialized.protocolVersion,
      loadSession: seen.initialized.agentCapabilities?.loadSession ?? false,
      listed: seen.listed
        ? seen.listed.map((info) => ({ session: info.sessionId, title: info.title ?? null, cwd: info.cwd }))
        : null,
      session: seen.session.sessionId,
      replayed: replayed
        ? replayed.map((update) => ({
            kind: update?.sessionUpdate ?? null,
            text: update?.content?.type === "text" ? update.content.text : "",
          }))
        : null,
      stop: seen.answered.stopReason,
      updates: updates.length,
      asked,
      files,
      onScreen: hasBuffer ? onScreen : null,
      raw: process.env.SWEM_ACP_VERBOSE ? updates : undefined,
      text,
    }),
  );
  swem.kill("SIGTERM");
  process.exit(0);
} catch (error) {
  clearTimeout(timeout);
  console.error(`the editor could not talk to SWEM: ${error?.message ?? error}`);
  console.error(`details: ${JSON.stringify(error?.data ?? error, null, 2)}`);
  console.error(complaints.join(""));
  swem.kill("SIGKILL");
  process.exit(5);
}
