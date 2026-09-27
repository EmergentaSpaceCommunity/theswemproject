// One content block of the protocol as a line a person reads.
import assert from "node:assert/strict";
import { test } from "node:test";

import { contentText } from "../src/agent/events.ts";

test("a block is said in a line, whatever kind it is", () => {
  assert.equal(contentText({ type: "text", text: "hello" }), "hello");
  assert.equal(contentText({ type: "resource_link", name: "notes.md", uri: "file:///w/notes.md" }), "↗ notes.md");
  assert.equal(contentText({ type: "resource", resource: { uri: "file:///w/outbox/report.pdf" } }), "Attachment · report.pdf");
  assert.equal(contentText({ type: "image" }), "Image attachment");
  assert.equal(contentText({ type: "something-new" }), "");
  assert.equal(contentText(null), "");
});
