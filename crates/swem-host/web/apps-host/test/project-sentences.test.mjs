// What the rail's rows look like by the time a person reads them. The Cycle
// writes them out of records, so every one of these asks the same question:
// is what is left something a person could have written?
import assert from "node:assert/strict";
import { test } from "node:test";

import { outputName, readable, unmarked } from "../src/project/sentences.ts";

test("a record ref in a sentence becomes the thing it names", () => {
  assert.equal(
    readable("Head revision `3aa7f1c179d5` has no delivery selection"),
    "Head revision this revision has no delivery selection",
  );
});

// The Cycle carries the module's own sentence on the requirement row now, so
// a loaded module's predicate reaches the rail already in words and these two
// are the fallback: what is left to read when the module that published the
// predicate is not loaded in the process that derived the ladder.
test("a predicate contract ref left in a sentence is still unwrapped", () => {
  assert.equal(
    readable(
      "the delivery of `music` was refused: its evidence says swem.predicate:sample-peak-abs-le@0.1 does not hold",
    ),
    'the delivery of `music` was refused: its evidence says "sample peak abs le" does not hold',
  );
});

test("a version with more parts than two still comes off", () => {
  assert.equal(readable("owes swem.predicate:artifact-reproduced@0.1.3"), 'owes "artifact reproduced"');
});

test("two predicates in one sentence are both read", () => {
  assert.equal(
    readable("swem.predicate:duration-ticks-within@0.1 and swem.predicate:artifact-reproduced@0.1"),
    '"duration ticks within" and "artifact reproduced"',
  );
});

test("a sentence with nothing to unwrap is left exactly as it was", () => {
  const plain = "nothing in `music` is chosen for delivery";
  assert.equal(readable(plain), plain);
});

test("an output root reads as the module's own words for it", () => {
  assert.equal(outputName("swem.output:audio-master"), "audio master");
  assert.equal(outputName("plain"), "plain");
});

// A tooltip is the one place a marked name cannot be drawn as one: browsers
// render `title` as plain text, so the marks arrive as punctuation meaning
// nothing. The music thread found this shape in its own workbench header on
// 2026-09-21 and the same shape was here, on two tooltips.
test("a sentence for a tooltip keeps the names and drops the marks", () => {
  assert.equal(
    unmarked("the delivery of `music` was refused"),
    "the delivery of music was refused",
  );
});

test("a sentence with no marks is unchanged", () => {
  assert.equal(unmarked("nothing is owed by the records"), "nothing is owed by the records");
});

// A lone backtick is somebody's punctuation, not half a name: `marked` only
// pairs them, so this must not eat the character or the rest of the line.
test("an unpaired mark is left where it is", () => {
  assert.equal(unmarked("a `lone mark stays"), "a `lone mark stays");
});
