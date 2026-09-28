// A name after `@` stands out where it is one of those in the chat.

import assert from "node:assert/strict";
import { test } from "node:test";

import { withNames } from "../src/page/names.ts";

test("names of those in the chat are set apart, and nothing else is", () => {
  const handles = ["ada", "builder"];
  assert.deepEqual(withNames("@ada draft the notes, @builder check the build.", handles), [
    { name: "ada" },
    " draft the notes, ",
    { name: "builder" },
    " check the build.",
  ]);
  // Somebody who is not in the chat, an address and a path name nobody.
  assert.deepEqual(withNames("ask @marta, mail ada@example.com, see src/@ada/x", handles), ["ask @marta, mail ada@example.com, see src/@ada/x"]);
  assert.deepEqual(withNames("(@ada)", handles), ["(", { name: "ada" }, ")"]);
  assert.deepEqual(withNames("nothing here", handles), ["nothing here"]);
});
