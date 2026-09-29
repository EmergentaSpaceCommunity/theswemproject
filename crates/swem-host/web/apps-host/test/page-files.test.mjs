// Places in an agent's folder, and what a file is called and coloured as.

import assert from "node:assert/strict";
import { test } from "node:test";

import { ago, folderOf, languageOf, nameOf, sized, whyNotNamed, within } from "../src/page/files.ts";

test("a place is a name in a folder, and the folder it works in is the empty place", () => {
  assert.equal(within("", "README.md"), "README.md");
  assert.equal(within("src/page", "files.ts"), "src/page/files.ts");
  assert.equal(folderOf("src/page/files.ts"), "src/page");
  assert.equal(folderOf("README.md"), "");
  assert.equal(nameOf("src/page/files.ts"), "files.ts");
  assert.equal(nameOf("README.md"), "README.md");
});

test("a name that is a place or holds a slash is refused in words", () => {
  assert.equal(whyNotNamed("notes.md"), "");
  assert.equal(whyNotNamed(".env"), "");
  assert.notEqual(whyNotNamed(""), "");
  assert.notEqual(whyNotNamed("  "), "");
  assert.notEqual(whyNotNamed(".."), "");
  assert.notEqual(whyNotNamed("a/b"), "");
  assert.notEqual(whyNotNamed("a\\b"), "");
  assert.notEqual(whyNotNamed("x".repeat(256)), "");
});

test("a file is coloured by what its name ends with", () => {
  assert.equal(languageOf("src/main.rs"), "rust");
  assert.equal(languageOf("page/Files.TSX"), "tsx");
  assert.equal(languageOf("notes.md"), "markdown");
  assert.equal(languageOf("compose.yaml"), "yaml");
  assert.equal(languageOf("Makefile"), null);
  assert.equal(languageOf("a.b/c"), null);
});

test("a size reads as a person says it", () => {
  assert.equal(sized(12), "12 B");
  assert.equal(sized(2048), "2 KB");
  assert.equal(sized(3 * 1024 * 1024), "3.0 MB");
});

test("when something was changed reads as a person says it", () => {
  const now = 1_790_000_000_000;
  assert.equal(ago(now - 20_000, now), "just now");
  assert.equal(ago(now - 60_000, now), "a minute ago");
  assert.equal(ago(now - 4 * 60_000, now), "4 minutes ago");
  assert.equal(ago(now - 60 * 60_000, now), "an hour ago");
  assert.equal(ago(now - 5 * 3600_000, now), "5 hours ago");
  assert.equal(ago(now - 26 * 3600_000, now), "yesterday");
  assert.equal(ago(now - 3 * 86_400_000, now), "3 days ago");
  assert.equal(ago(now + 5000, now), "just now");
});
