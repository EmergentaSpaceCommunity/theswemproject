// What runs in a terminal, as a person would say it.

import assert from "node:assert/strict";
import { test } from "node:test";

import { running } from "../src/page/terminals.ts";

test("a program is called by its name, with what it was given", () => {
  assert.equal(running({ command: "/bin/zsh" }), "zsh");
  assert.equal(running({ command: "/usr/bin/npm", args: ["test"] }), "npm test");
});

test("a shell given one line to run is that line", () => {
  assert.equal(running({ command: "/bin/sh", args: ["-c", "echo done; sleep 20"] }), "echo done; sleep 20");
  assert.equal(running({ command: "/bin/bash", args: ["-lc", "cargo test"] }), "cargo test");
});
