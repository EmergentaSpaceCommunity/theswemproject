// The Agent space's state machine, driven by a fake host.
//
// The scenario is the one the owner could not get past: an agent that needs
// a key. The store must learn that BEFORE offering to start, take the key,
// send it to the host under the variable the agent named, start with that
// method, and report the session in words - and when a session does fail,
// say why in a sentence while keeping the exact reason.
import assert from "node:assert/strict";
import { test } from "node:test";

import { SessionStore } from "../src/agent/session.ts";

/// A host that answers the routes the store uses, and records what it saw.
function fakeHost(options = {}) {
  const seen = [];
  const secrets = [];
  let events = [];
  let connectionOpened = false;
  // The record the rail's card is a read of, and the phase the page polls.
  // Both move when something happens to the session, which is the point of
  // the dying-turn test below.
  let phase = { phase: "idle", session_id: "s1" };
  let model = "fast";
  let record = [{ route_id: "route-1", agent_id: "fixture", events: 0 }];
  const routes = {
    "GET /api/profiles": () => [{ profile_id: "p1", agent_id: "fixture" }],
    "GET /api/profiles/p1/handshake": () => ({
      agent_name: "fixture",
      agent_version: "0",
      readiness: "handshake_ready",
      auth_methods: [{ type: "env_var", id: "api-key", name: "API key", vars: [{ name: "FIXTURE_KEY", secret: true }] }],
    }),
    "GET /api/profiles/p1/secrets": () => ({ types: [{ type_id: "generic_env_var", label: "Environment variable", env_var: null, hint: "" }], secrets }),
    "PUT /api/profiles/p1/secrets": (body) => {
      secrets.push({ name: body.name, type_id: body.type_id, label: body.label });
      return { types: [], secrets };
    },
    "POST /api/connections": (body) => {
      if (!secrets.some((secret) => secret.name === "FIXTURE_KEY")) {
        return { status: 502, body: { error: "agent requires authentication before a session can start: Authentication required" } };
      }
      connectionOpened = true;
      return { connection_id: body.connection_id, route_id: "route-1" };
    },
    "GET /api/connections/wb": () => ({ phase }),
    "GET /api/profiles/p1/sessions": () => record,
    "GET /api/routes/route-1/events": () => {
      if (events.length === 0) return null;
      const batch = { events };
      events = [];
      return batch;
    },
    "POST /api/routes/route-1/ack": () => ({ acknowledged: true }),
    "GET /api/connections/wb/permissions/next": () => null,
    "GET /api/connections/wb/elicitations/next": () => null,
    "GET /api/connections/wb/apps/observations/next": () => null,
    "GET /api/connections/wb/apps": () => [],
    // What the fixture agent lets a session configure: a model with two
    // choices, as the host relays it.
    "GET /api/connections/wb/options": () => ({
      config_options: [{ id: "model", name: "Model", category: "model", type: "select", currentValue: model, options: [
        { value: "fast", name: "Fast" }, { value: "quality", name: "Quality" },
      ] }],
      legacy_modes: null,
    }),
    "POST /api/connections/wb/options": (body) => {
      if (body.config_id !== "model" || !["fast", "quality"].includes(body.value)) {
        return { status: 409, body: { error: `agent did not expose config option value \`${body.value}\`` } };
      }
      model = body.value;
      return { config_options: [{ id: "model", name: "Model", category: "model", type: "select", currentValue: model, options: [
        { value: "fast", name: "Fast" }, { value: "quality", name: "Quality" },
      ] }] };
    },
    "POST /api/connections/wb/close": () => ({ termination: "closed" }),
    "POST /api/connections/wb/prompt": () => {
      events.push({ sequence: 1, kind: "host/prompt_submitted", source: "host", payload: { content: [{ type: "text", text: "Привет" }] } });
      events.push({ sequence: 2, kind: "acp/session_update", source: "native_live", payload: { update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "Hello" } } } });
      events.push({ sequence: 3, kind: "acp/prompt_response", source: "native_live", payload: { stop_reason: "end_turn", control_outcome: "completed" } });
      return { stop_reason: "end_turn", control_outcome: "completed" };
    },
  };
  if (options.turnDies) {
    routes["POST /api/connections/wb/prompt"] = () => {
      events.push({ sequence: 1, kind: "host/prompt_submitted", source: "host", payload: { content: [{ type: "text", text: "Привет" }] } });
      // The agent dies here. There is no `acp/prompt_response`, so the turn
      // never ends - but the record did change, and the session stopped.
      record = [{ route_id: "route-1", agent_id: "fixture", events: 1, opened_with: "Привет", last_kind: "host/prompt_submitted" }];
      phase = { phase: "finished", session_id: "s1" };
      return { status: 502, body: { error: "the agent stopped answering" } };
    };
  }
  const fetch = async (input, init = {}) => {
    const method = init.method ?? "GET";
    const url = new URL(input, "http://host");
    // Connection ids are minted per open; normalise them for the table.
    const path = url.pathname.replace(/wb-[0-9a-z-]+/g, "wb").replace(/\/wb$/, "/wb");
    seen.push(`${method} ${path}`);
    const handler = routes[`${method} ${path}`];
    if (!handler) return new Response(JSON.stringify({ error: `no route ${method} ${path}` }), { status: 404 });
    const body = init.body ? JSON.parse(init.body) : undefined;
    const answer = handler(body);
    if (answer && typeof answer === "object" && "status" in answer && "body" in answer) {
      return new Response(JSON.stringify(answer.body), { status: answer.status });
    }
    // Long polls answer immediately here; the store's loops still spin, so
    // a null answer is delayed a little to keep the test from busy-looping.
    if (answer === null) {
      await new Promise((resolve) => setTimeout(resolve, 20));
      // An empty events poll answers the shape the host uses, not null.
      if (path.endsWith("/events")) return new Response(JSON.stringify({ events: [] }), { status: 200 });
    }
    return new Response(JSON.stringify(answer), { status: 200 });
  };
  return { fetch, seen, secrets, opened: () => connectionOpened };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 60));

test("the store learns what the agent will ask for before offering to start", async () => {
  const host = fakeHost();
  const store = new SessionStore(host.fetch);
  await store.loadProfiles();
  await settle();
  const methods = store.authMethods();
  assert.equal(methods.length, 1);
  assert.equal(methods[0].type, "env_var");
  assert.deepEqual(methods[0].vars.map((v) => v.name), ["FIXTURE_KEY"]);
});

test("a session that fails for want of a key says so in a sentence, and keeps the reason", async () => {
  const host = fakeHost();
  const store = new SessionStore(host.fetch);
  await store.loadProfiles();
  await store.open("new", "api-key");
  const state = store.getSnapshot();
  assert.equal(state.connectionId, null);
  assert.ok(state.problem.startsWith("The session could not start:"), state.problem);
  assert.ok(state.problem.includes("Authentication required"), "the exact reason is kept");
  assert.ok(!state.problem.includes("{"), "no JSON reached the person");
});

test("the key a person types reaches the host under the variable the agent named, and then the session starts", async () => {
  const host = fakeHost();
  const store = new SessionStore(host.fetch);
  await store.loadProfiles();
  await store.saveSecret("p1", { type_id: "generic_env_var", label: "Fixture key", name: "FIXTURE_KEY", value: "s3cr3t" });
  assert.deepEqual(host.secrets.map((s) => s.name), ["FIXTURE_KEY"]);
  assert.ok(!JSON.stringify(store.getSnapshot()).includes("s3cr3t"), "the value is not kept in the store");

  await store.open("new", "api-key");
  assert.ok(host.opened(), "the connection did not open");
  const openCall = host.seen.find((call) => call.startsWith("POST /api/connections"));
  assert.ok(openCall);
  await settle();
  assert.equal(store.getSnapshot().phase?.phase, "idle");

  await store.sendPrompt({ text: "Привет", files: [] });
  await settle();
  const conversation = store.getSnapshot().conversation;
  const messages = conversation.items.filter((item) => item.kind === "message").map((item) => item.message.text);
  assert.deepEqual(messages, ["Привет", "Hello"]);
  assert.equal(conversation.turnOutcome, "turn: end_turn / completed");

  await store.close();
  const ended = store.getSnapshot();
  assert.equal(ended.problem, "");
  assert.equal(ended.connectionId, null);
  assert.equal(ended.conversation.terminal?.summary, "Session ended (closed).");
});

test("a turn that dies without ending refreshes the record the rail's card is a read of", async () => {
  const host = fakeHost({ turnDies: true });
  const store = new SessionStore(host.fetch);
  await store.loadProfiles();
  await store.saveSecret("p1", { type_id: "generic_env_var", label: "Fixture key", name: "FIXTURE_KEY", value: "s3cr3t" });
  await store.open("new", "api-key");
  // Whatever this test finds, the session's loops have to stop, or the run
  // never ends: a failed assertion would otherwise leave them polling.
  try {
    await settle();
    assert.equal(store.getSnapshot().phase?.phase, "idle");
    assert.deepEqual(store.getSnapshot().sessions.map((s) => s.events), [0], "the card as the session opened");

    await assert.rejects(store.sendPrompt({ text: "Привет", files: [] }));
    await settle();
    // The turn never ended: there is no `acp/prompt_response` to end it, which
    // is exactly what used to leave the card saying nothing had been said.
    assert.equal(store.getSnapshot().conversation.turnEnded, false, "the turn did not end");

    // The phase is polled every 750ms, so give it one poll to notice.
    await new Promise((resolve) => setTimeout(resolve, 1200));
    assert.equal(store.getSnapshot().phase?.phase, "finished");
    assert.deepEqual(
      store.getSnapshot().sessions.map((session) => session.opened_with),
      ["Привет"],
      "the card was not re-read when the session stopped",
    );
  } finally {
    await store.close();
  }
});

test("the session's options are read from the host when it opens, and a choice goes back to the agent", async () => {
  const host = fakeHost();
  const store = new SessionStore(host.fetch);
  await store.loadProfiles();
  await store.saveSecret("p1", { type_id: "generic_env_var", label: "Fixture key", name: "FIXTURE_KEY", value: "s3cr3t" });
  await store.open("new", "api-key");
  try {
    await settle();
    const options = store.getSnapshot().sessionOptions;
    assert.ok(options, "the options were read when the session opened");
    const model = options.options.find((option) => option.category === "model");
    assert.equal(model.currentValue, "fast");
    assert.deepEqual(model.choices.map((choice) => choice.value), ["fast", "quality"]);

    await store.setSessionOption("model", "quality");
    assert.equal(store.getSnapshot().sessionOptions.options[0].currentValue, "quality");
    assert.equal(store.getSnapshot().problem, "");

    await store.setSessionOption("model", "imaginary");
    assert.ok(store.getSnapshot().problem.includes("imaginary"), "the agent's refusal is the problem line");
    assert.equal(store.getSnapshot().sessionOptions.options[0].currentValue, "quality", "a refused choice changes nothing");
  } finally {
    await store.close();
  }
});
