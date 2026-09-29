// What was found in an agent's machine, as a person reads it.

import assert from "node:assert/strict";
import { test } from "node:test";

import { programsFound, took, versionIn } from "../src/page/looks.ts";

test("how long something took is said in the unit a person would use", () => {
  assert.equal(took(40), "40 ms");
  assert.equal(took(820), "820 ms");
  assert.equal(took(980), "1.0 s");
  assert.equal(took(1240), "1.2 s");
  assert.equal(took(12_000), "12.0 s");
});

test("a version is the number in what a program says of itself", () => {
  assert.equal(versionIn("git version 2.47.0"), "2.47.0");
  assert.equal(versionIn("v22.11.0"), "22.11.0");
  assert.equal(versionIn("Python 3.12.4"), "3.12.4");
  assert.equal(versionIn("podman version 4.9.5"), "4.9.5");
  assert.equal(versionIn(""), "");
  assert.equal(versionIn(null), "");
});

test("only the programs that are there are named", () => {
  const programs = [
    { name: "git", path: "/usr/bin/git", version: "git version 2.47.0" },
    { name: "node", path: null, version: null },
    { name: "python3", path: "/usr/bin/python3", version: "Python 3.12.4" },
  ];
  assert.deepEqual(programsFound(programs, ["git", "node", "python3"]), { names: "git, python", versions: "2.47.0 · 3.12.4" });
  assert.deepEqual(programsFound(programs, ["node"]), { names: "", versions: "" });
});
