// The browser walks fail when a View calls the host before its handshake is
// done. That check is only as good as the sentence it looks for, and that
// sentence belongs to the vendored bridge, not to us: an ext-apps upgrade that
// rewords the warning would disarm the check silently, and every walk would go
// on passing while saying nothing about the race. So the wording is asserted
// here, where an upgrade is reviewed.
import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import {dirname, join} from "node:path";

import {HANDSHAKE_WARNING, handshakeViolations} from "../../../tests/app_handshake.mjs";

const here = dirname(fileURLToPath(import.meta.url));

test("the bundled bridge still warns in the words the walks look for", () => {
  const bundle = readFileSync(join(here, "..", "dist", "apps-bridge.js"), "utf8");
  assert.ok(
    bundle.includes(HANDSHAKE_WARNING),
    `the shipped bridge no longer says ${JSON.stringify(HANDSHAKE_WARNING)}: ` +
      "either it stopped warning about pre-handshake calls, or it reworded the warning. " +
      "Until this matches again, no browser walk can see that race.",
  );
});

test("the check names the pre-handshake calls and nothing else", () => {
  const lines = [
    "[page] console.warning: [ext-apps] AppBridge received 'tools/call' before ui/notifications/initialized. " +
      "The View is calling host methods before completing the handshake; it should await app.connect() first.",
    "[page] console.error: something else entirely",
    "[frame] console.warning: [ext-apps] AppBridge received a second ui/initialize.",
  ];
  assert.deepEqual(handshakeViolations(lines), [lines[0]]);
});
