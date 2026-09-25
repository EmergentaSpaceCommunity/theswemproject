// The Cycle marks a logical id with backticks, and the rail used to print
// the marks. A person who has never written a backtick reads punctuation
// the sentence does not have: "the delivery of `music` was refused".
import assert from "node:assert/strict";
import { test } from "node:test";

import { marked } from "../src/project/sentences.ts";

test("a marked name comes out as a name, and the prose around it as prose", () => {
  assert.deepEqual(marked("the delivery of `music` was refused"), [
    { text: "the delivery of ", name: false },
    { text: "music", name: true },
    { text: " was refused", name: false },
  ]);
});

test("a sentence with nothing marked is one part", () => {
  assert.deepEqual(marked("Releasing"), [{ text: "Releasing", name: false }]);
  assert.deepEqual(marked(""), []);
});

test("two names in one sentence, and a mark at either end", () => {
  assert.deepEqual(marked("`music` needs `artwork`"), [
    { text: "music", name: true },
    { text: " needs ", name: false },
    { text: "artwork", name: true },
  ]);
});

test("a lone backtick is not a mark, so the sentence keeps it", () => {
  assert.deepEqual(marked("nothing in `music is chosen"), [
    { text: "nothing in `music is chosen", name: false },
  ]);
});

test("whatever it is given, the text a person reads is the text that went in, minus the marks", () => {
  for (const sentence of [
    "the delivery of `music` was refused: its evidence says X does not hold",
    "Releasing `music`",
    "nothing in `music` is chosen for delivery",
    "no intent text recorded",
  ]) {
    assert.equal(
      marked(sentence).map((part) => part.text).join(""),
      sentence.replace(/`([^`]*)`/g, "$1"),
    );
  }
});
