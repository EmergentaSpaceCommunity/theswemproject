// What the person sees in the page after they worked from their editor.
//
// The editor is a second door onto one product, not a second product: the
// conversation it held is in the same record the page reads, and the file the
// agent wrote is in the same Files panel. This driver looks for both, through
// the product's own door, the way a person would.

import {launchBrowser, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, profile, wrote, editor] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!url || !profile || !wrote || !editor) {
  console.error("usage: editor_door_record_driver.mjs <url> <profile> <file> <editor name>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "editor-record"});

step("the product opens");
await b.waitFor("the shell", async () => b.exists("#space-agent"));
await b.click("#space-agent");
await b.waitFor("the profiles", async () =>
  b.evaluate(`document.querySelectorAll("#profiles option").length > 0`), 300);
await b.evaluate(`(() => {
  const picker = document.getElementById("profiles");
  picker.value = ${JSON.stringify(profile)};
  picker.dispatchEvent(new Event("change", {bubbles: true}));
})()`);

// The session the editor opened is this person's session, listed beside any
// they started themselves.
await b.waitFor("the editor's session is in the list", async () =>
  b.evaluate(`document.querySelectorAll("#sessions .session-row").length > 0`), 300);
const listed = await b.evaluate(
  `JSON.stringify([...document.querySelectorAll("#sessions .session-row")].map(row => row.dataset.routeId))`,
).then(JSON.parse);
step(`the page lists ${listed.length} session(s) for this profile`);

// And the record says who wrote the turn: the editor, by the name it gave at
// initialize, on the surface an editor writes from.
const record = await b.evaluate(`(async () => {
  const sessions = await (await fetch("/api/profiles/" + encodeURIComponent(${JSON.stringify(profile)}) + "/sessions")).json();
  const route = (Array.isArray(sessions) ? sessions : sessions.sessions || [])[0]?.route_id;
  const history = await (await fetch("/api/routes/" + encodeURIComponent(route) + "/history?limit=200")).json();
  const written = (history.events || []).filter((event) => event.kind === "host/turn_written");
  return JSON.stringify({route, written});
})()`).then(JSON.parse);
const wrote_it = record.written[0]?.payload?.correspondent;
if (wrote_it?.surface !== "editor" || wrote_it?.author !== editor) {
  cleanup(1, `the record does not say the editor wrote the turn: ${JSON.stringify(record)}`);
}
step(`the lane says who wrote it (${wrote_it.author} from ${wrote_it.surface})`);

// The file that turn produced is in the person's own Files, because the agent
// worked in the profile's directory and not in the one the editor claimed.
await b.click('[data-agent-tab="files"]');
await b.waitFor("the Files panel", async () => b.exists('[data-agent-panel="files"]:not([hidden])'));
await b.waitFor("the editor's file is listed", async () => {
  await b.click("#files-refresh");
  return b.exists(`[data-files-area="outbox"] [data-file-name=${JSON.stringify(wrote)}]`);
}, 30, 500);
step("the file the editor's turn wrote is in the person's Files");

console.log("editor record OK");
console.log(JSON.stringify({sessions: listed.length, route: record.route, wrote: wrote_it}));
cleanup(0, "editor record done");
