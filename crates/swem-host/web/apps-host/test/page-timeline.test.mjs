// A chat as a person reads it, from what happened in it.

import assert from "node:assert/strict";
import { test } from "node:test";

import { emptyTimeline, grouped, takeIn, toolCalled } from "../src/page/timeline.ts";

const update = (sequence, agent, body) => ({
  sequence, chat_id: "c", agent_id: agent, kind: "acp/session_update", source: "native_live", payload: { update: body },
});
const chunk = (sequence, agent, text) => update(sequence, agent, { sessionUpdate: "agent_message_chunk", content: { type: "text", text } });
const said = (sequence, sender, text, kind = "chat/message") => ({
  sequence, chat_id: "c", kind, source: "host", payload: {},
  message: { message_id: `m${sequence}`, sequence, chat_id: "c", sender_id: sender, channel: "workbench", content: [{ type: "text", text }], text, named: [] },
});
const read = (events) => events.reduce(takeIn, emptyTimeline());
const shape = (timeline) => timeline.items.map((item) => `${item.kind}:${item.by ?? ""}:${item.text ?? item.message?.text ?? item.tool?.title ?? item.question?.state ?? ""}`);

test("what an agent says between what it does is drawn in the order it happened", () => {
  const timeline = read([
    said(1, "me", "draft it"),
    { sequence: 2, chat_id: "c", kind: "chat/delivery", source: "host", payload: { delivery_id: "d", agent_id: "ada", message_id: "m1", state: "queued" } },
    // The message being given to the agent is not a second message.
    { sequence: 3, chat_id: "c", agent_id: "ada", kind: "host/prompt_submitted", source: "host", payload: { content: [{ type: "text", text: "draft it" }] } },
    chunk(4, "ada", "Reading"),
    chunk(5, "ada", " them."),
    update(6, "ada", { sessionUpdate: "tool_call", toolCallId: "t1", title: "Read 14 pull requests", status: "pending" }),
    update(7, "ada", { sessionUpdate: "tool_call_update", toolCallId: "t1", status: "completed" }),
    chunk(8, "ada", "Done."),
    { ...said(9, "ada", "Reading them.Done.", "acp/prompt_response"), agent_id: "ada" },
  ]);
  assert.deepEqual(shape(timeline), ["said:me:draft it", "text:ada:Reading them.", "tool:ada:Read 14 pull requests", "text:ada:Done."]);
  assert.equal(timeline.items[2].tool.status, "completed");
  assert.ok(timeline.items.every((item) => item.kind !== "text" || item.settled), "the turn ended, so nothing is still being said");
  assert.equal(timeline.through, 9);
  const groups = grouped(timeline.items);
  assert.deepEqual(groups.map((group) => [group.by, group.items.length]), [["me", 1], ["ada", 3]]);
});

test("told the same thing twice, it is taken in once", () => {
  const events = [said(1, "me", "hello"), chunk(2, "ada", "Hi")];
  const once = read(events);
  assert.deepEqual(read([...events, ...events]), once);
  assert.equal(once.items[1].settled, false, "it is still speaking");
});

test("several speak in one chat and each keeps its own words", () => {
  const timeline = read([
    said(1, "me", "@ada and @builder, go"),
    chunk(2, "ada", "A1 "),
    chunk(3, "builder", "B1 "),
    chunk(4, "ada", "A2"),
    chunk(5, "builder", "B2"),
    said(6, "clock", "how are we doing"),
  ]);
  assert.deepEqual(shape(timeline), ["said:me:@ada and @builder, go", "text:ada:A1 A2", "text:builder:B1 B2", "said:clock:how are we doing"]);
});

test("what an engine replayed is not said a second time", () => {
  const timeline = read([chunk(1, "ada", "Hello"), { ...chunk(2, "ada", "Hello"), source: "native_replay" }]);
  assert.deepEqual(shape(timeline), ["text:ada:Hello"]);
});

test("a question is drawn where it was asked and changes where it stands", () => {
  const question = (sequence, state, answer) => ({
    sequence, chat_id: "c", kind: "chat/question", source: "host",
    payload: { question_id: "q", agent_id: "ada", delivery_id: "d", kind: "permission", state, asked: { title: "Run git push", options: [] }, answer },
  });
  const asked = read([chunk(1, "ada", "I will push."), question(2, "waiting")]);
  assert.deepEqual(shape(asked), ["text:ada:I will push.", "ask:ada:waiting"]);
  assert.equal(asked.items[1].question.chat_id, "c");
  const answered = takeIn(takeIn(asked, question(3, "answered", { option: "once", name: "Allow once" })), chunk(4, "ada", "Pushed."));
  assert.deepEqual(shape(answered), ["text:ada:I will push.", "ask:ada:answered", "text:ada:Pushed."]);
  assert.equal(answered.items[1].question.answer.name, "Allow once");
});

test("how a turn ended is said when it did not end by itself", () => {
  const delivery = (sequence, state, outcome) => ({
    sequence, chat_id: "c", kind: "chat/delivery", source: "host",
    payload: { delivery_id: "d", agent_id: "ada", message_id: "m1", state, outcome },
  });
  const running = read([said(1, "me", "go"), delivery(2, "queued"), delivery(3, "running"), chunk(4, "ada", "Work")]);
  assert.deepEqual(shape(running), ["said:me:go", "text:ada:Work"]);
  assert.deepEqual(shape(takeIn(running, delivery(5, "stopped"))).at(-1), "note:ada:Stopped.");
  assert.deepEqual(shape(takeIn(running, delivery(5, "failed", "the agent is not signed in"))).at(-1), "note:ada:the agent is not signed in");
  const done = takeIn(running, delivery(5, "done"));
  assert.deepEqual(shape(done), ["said:me:go", "text:ada:Work"]);
  assert.equal(done.items[1].settled, true);
});

test("what changed about the agent since the chat began is said in words", () => {
  const timeline = read([
    { sequence: 1, chat_id: "c", agent_id: "ada", kind: "host/setup_changed", source: "host", payload: { in_words: ["it works somewhere else now", "it now reaches notes"] } },
    { sequence: 2, chat_id: "c", agent_id: "ada", kind: "host/attachment_unavailable", source: "host", payload: { in_words: ["music"] } },
  ]);
  assert.deepEqual(timeline.items.map((item) => item.text), [
    "Since this chat began: it works somewhere else now; it now reaches notes.",
    "Attached, but not set up on this computer: music. It works without.",
  ]);
});

test("an App is brought by the call that begins, not by how it ended", () => {
  const observed = (sequence, payload) => ({ sequence, chat_id: "c", agent_id: "ada", kind: "host/app_tool_observed", source: "host", payload });
  const begun = read([observed(4, { observation_id: "o2", cursor: 2, phase: "request" })]);
  assert.deepEqual(begun.brought, { ada: { place: 4, call: 2 } });
  const ended = takeIn(begun, observed(5, { observation_id: "o2", phase: "response", status: "completed" }));
  assert.deepEqual(ended.brought, { ada: { place: 4, call: 2 } });
});

test("a tool of a server is called by what it does and whose it is", () => {
  assert.equal(toolCalled("mcp__swem-time__make_schedule"), "make schedule · swem-time");
  assert.equal(toolCalled("mcp__notes__save_note"), "save note · notes");
  assert.equal(toolCalled("Read 14 pull requests"), "Read 14 pull requests");
  assert.equal(toolCalled(undefined), undefined);
  const timeline = [update(1, "ada", { sessionUpdate: "tool_call", toolCallId: "t1", title: "mcp__swem-time__make_schedule", status: "pending" })].reduce(takeIn, emptyTimeline());
  assert.equal(timeline.items[0].tool.title, "make schedule · swem-time");
});
