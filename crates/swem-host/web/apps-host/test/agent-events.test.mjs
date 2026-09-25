// What a person sees is a function of the events the host delivered. These
// tests drive that function directly, with no DOM, and ask the questions the
// owner's live run raised: is a failure a sentence, and is the reason kept.
import assert from "node:assert/strict";
import { test } from "node:test";

import { describeTerminal, emptyConversation, reduceEvent } from "../src/agent/events.ts";

const event = (kind, payload, sequence = 1) => ({ sequence, kind, source: "host", payload });

test("a failed terminal is a sentence, and the reason is kept beside it", () => {
  const terminal = describeTerminal({
    status: "failed",
    error: "agent requires authentication before a session can start: Authentication required",
  });
  assert.equal(terminal.status, "failed");
  assert.equal(terminal.needsAuthentication, true);
  assert.ok(!terminal.summary.includes("{"), "the summary is not JSON");
  assert.ok(terminal.summary.toLowerCase().includes("authenticated"));
  assert.ok(terminal.detail.includes("Authentication required"), "the exact reason is not lost");
});

test("an ordinary end of session is not called a failure", () => {
  const terminal = describeTerminal({ status: "finished", termination: "closed" });
  assert.equal(terminal.status, "finished");
  assert.equal(terminal.needsAuthentication, false);
  assert.match(terminal.summary, /ended/);
});

test("the agent's answer streams into one message rather than one per chunk", () => {
  let conversation = emptyConversation();
  conversation = reduceEvent(conversation, event("host/prompt_submitted", { content: [{ type: "text", text: "Привет" }] }));
  for (const text of ["Hel", "lo ", "there"]) {
    conversation = reduceEvent(
      conversation,
      event("acp/session_update", { update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text } } }),
    );
  }
  const messages = conversation.items.filter((item) => item.kind === "message").map((item) => item.message);
  assert.deepEqual(messages.map((message) => [message.role, message.text]), [
    ["user", "Привет"],
    ["assistant", "Hello there"],
  ]);
});

test("a turn's end names its outcome and closes the permission dialog", () => {
  let conversation = emptyConversation();
  conversation = reduceEvent(conversation, event("acp/prompt_response", { stop_reason: "end_turn", control_outcome: "completed" }));
  assert.equal(conversation.turnOutcome, "turn: end_turn / completed");
  assert.equal(conversation.turnEnded, true);
  assert.equal(conversation.streaming, null);
});

test("a thought is shown quietly, and a tool the agent uses is a card that follows the call", () => {
  let conversation = emptyConversation();
  conversation = reduceEvent(conversation, event("acp/session_update", { update: { sessionUpdate: "agent_thought_chunk", content: { type: "text", text: "hmm" } } }));
  conversation = reduceEvent(conversation, event("acp/session_update", { update: { sessionUpdate: "tool_call", toolCallId: "c1", title: "read_file", kind: "read", status: "pending" } }));
  const [thought, tool] = conversation.items;
  assert.equal(thought.message.role, "thought");
  assert.equal(thought.message.quiet, true);
  assert.equal(tool.kind, "tool");
  assert.equal(tool.tool.title, "read_file");
  assert.equal(tool.tool.status, "pending");
  conversation = reduceEvent(conversation, event("acp/session_update", { update: {
    sessionUpdate: "tool_call_update", toolCallId: "c1", status: "completed",
    content: [{ type: "content", content: { type: "text", text: "42 lines" } }],
    locations: [{ path: "/w/notes.txt" }],
  } }));
  assert.equal(conversation.items.length, 2, "the update lands on the same card");
  assert.equal(conversation.items[1].tool.status, "completed");
  assert.deepEqual(conversation.items[1].tool.content, ["42 lines"]);
  assert.deepEqual(conversation.items[1].tool.locations, ["/w/notes.txt"]);
  // An update nobody announced still makes a card.
  conversation = reduceEvent(conversation, event("acp/session_update", { update: { sessionUpdate: "tool_call_update", toolCallId: "c2", status: "failed", title: "run" } }));
  assert.equal(conversation.items[2].tool.status, "failed");
});

test("the plan, the mode, the commands and the usage are the agent's own, read off its updates", () => {
  let conversation = emptyConversation();
  conversation = reduceEvent(conversation, event("acp/session_update", { update: { sessionUpdate: "plan", entries: [
    { content: "read the file", priority: "high", status: "completed" },
    { content: "fix it", status: "in_progress" },
    { content: "run tests", status: "weird" },
  ] } }));
  assert.deepEqual(conversation.plan.map((entry) => entry.status), ["completed", "in_progress", "pending"]);
  conversation = reduceEvent(conversation, event("acp/session_update", { update: { sessionUpdate: "plan", entries: [] } }));
  assert.equal(conversation.plan.length, 0, "a plan is replaced whole");
  conversation = reduceEvent(conversation, event("acp/session_update", { update: { sessionUpdate: "current_mode_update", currentModeId: "code" } }));
  assert.equal(conversation.mode, "code");
  conversation = reduceEvent(conversation, event("acp/session_update", { update: { sessionUpdate: "available_commands_update", availableCommands: [
    { name: "compact", description: "shorten the context" }, { name: "" },
  ] } }));
  assert.deepEqual(conversation.commands, [{ name: "compact", description: "shorten the context" }]);
  conversation = reduceEvent(conversation, event("acp/session_update", { update: { sessionUpdate: "usage_update", used: 1200, size: 200000, cost: 0.02, currency: "USD" } }));
  assert.equal(conversation.usage.used, 1200);
  assert.equal(conversation.usage.currency, "USD");
});

test("the session's opening and every options change ask the page to read the options again", () => {
  let conversation = emptyConversation();
  assert.equal(conversation.optionsVersion, 0);
  conversation = reduceEvent(conversation, event("acp/session_new", { sessionId: "s1", configOptions: [], modes: { currentModeId: "ask", availableModes: [] } }));
  assert.equal(conversation.optionsVersion, 1);
  assert.equal(conversation.mode, "ask");
  conversation = reduceEvent(conversation, event("acp/session_update", { update: { sessionUpdate: "config_option_update", configOptions: [] } }));
  assert.equal(conversation.optionsVersion, 2);
});

test("artifacts and their absence are both shown, and refusals are counted", () => {
  let conversation = emptyConversation();
  conversation = reduceEvent(conversation, event("host/artifact_available", { descriptor_id: "d1", name: "out.wav", media_type: "audio/wav", byte_length: 3, content_digest: "sha256:x" }));
  conversation = reduceEvent(conversation, event("host/artifact_unavailable", { name: "gone.bin", reason: "resource_link_unavailable" }));
  conversation = reduceEvent(conversation, event("host/app_tool_call", { decision: "refused" }));
  conversation = reduceEvent(conversation, event("host/app_tool_call", { decision: "allowed" }));
  assert.equal(conversation.items[0].kind, "artifact");
  assert.equal(conversation.items[1].kind, "unavailable");
  assert.equal(conversation.refusals, 1);
});
