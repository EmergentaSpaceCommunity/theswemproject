// What the door says of a device and of a token, and where Settings is.

import assert from "node:assert/strict";
import { test } from "node:test";

import { likelyName, mayInWords } from "../src/page/door.ts";
import { address, read } from "../src/page/place.ts";

test("a device is offered a name a person would give it", () => {
  assert.equal(likelyName("Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15"), "iPhone");
  assert.equal(likelyName("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/140 Safari/537.36"), "Mac");
  assert.equal(likelyName("Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 Chrome/140 Mobile Safari/537.36"), "Android phone");
  assert.equal(likelyName("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"), "Windows computer");
  assert.equal(likelyName("Mozilla/5.0 (X11; Linux x86_64) Gecko/20100101 Firefox/141.0"), "Linux computer");
  // Nothing is made up for a browser that does not say.
  assert.equal(likelyName("curl/8.7.1"), "");
});

test("what a token may do is said in words, the widest first", () => {
  assert.equal(mayInWords(["say-what-is-due"]), "May say that something is due, and nothing else");
  assert.equal(mayInWords(["everything"]), "May do what the page does");
  assert.equal(mayInWords(["say-what-is-due", "everything"]), "May do what the page does");
  assert.equal(mayInWords([]), "May do nothing");
});

test("Settings has an address a person can come back to", () => {
  assert.deepEqual(read("#/settings/access"), { at: "settings", tab: "access" });
  assert.deepEqual(read("#/settings"), { at: "settings", tab: "access" });
  assert.equal(address({ at: "settings", tab: "access" }), "#/settings/access");
});
