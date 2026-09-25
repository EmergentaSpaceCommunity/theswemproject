// The project's tabs and forms are functions of the envelope and the
// server's tool list; this asks those functions what a person would see.
import assert from "node:assert/strict";
import { test } from "node:test";

import { formSchemaFor, slotTabs, toolFor } from "../src/project/slots.ts";

const apps = [{ uri: "ui://x/one", description: "One" }, { uri: "ui://x/two" }];

test("one tab per slot, named by the slot, opening the slot's workbench", () => {
  const envelope = { slots: { slots: [
    { slot: "music", module: "one-module", app_uri: "ui://x/one" },
    { slot: "site", module: "two-module", app_uri: "ui://x/two" },
  ], kinds: [] } };
  const tabs = slotTabs(envelope, apps);
  assert.deepEqual(tabs.map((tab) => tab.slot), ["music", "site"]);
  assert.equal(tabs[0].uri, "ui://x/one");
  assert.equal(tabs[0].description, "One");
  assert.equal(tabs[1].description, null);
});

test("a slot of a module this build lacks is listed, not dropped", () => {
  const envelope = { slots: { slots: [{ slot: "place", module: "elsewhere", app_uri: null }], kinds: [] } };
  const [tab] = slotTabs(envelope, apps);
  assert.equal(tab.slot, "place");
  assert.equal(tab.uri, null);
});

test("no slots means no workbench tabs, whatever the server publishes", () => {
  assert.deepEqual(slotTabs({ slots: { slots: [], kinds: [] } }, apps), []);
  assert.deepEqual(slotTabs({}, apps), []);
  assert.deepEqual(slotTabs(null, apps), []);
});

test("the form asks only what the rung did not already decide", () => {
  const schema = {
    type: "object",
    properties: { type_ref: { type: "string" }, media_type: { type: "string" }, text: { type: "string" } },
    required: ["type_ref", "media_type", "text"],
  };
  const form = formSchemaFor(schema, { type_ref: "t", media_type: "m" });
  assert.deepEqual(Object.keys(form.properties), ["text"]);
  assert.deepEqual(form.required, ["text"]);
  const whole = formSchemaFor(schema, undefined);
  assert.deepEqual(Object.keys(whole.properties), ["type_ref", "media_type", "text"]);
});

test("a person may use a tool only when the server exposes it to Apps", () => {
  const tools = [
    { name: "open", input_schema: {}, visibility: ["model", "app"] },
    { name: "agent_only", input_schema: {}, visibility: ["model"] },
  ];
  assert.equal(toolFor(tools, "open")?.name, "open");
  assert.equal(toolFor(tools, "agent_only"), null);
  assert.equal(toolFor(tools, "missing"), null);
});
