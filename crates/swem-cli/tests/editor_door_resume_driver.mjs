// What the page shows after a person closed their editor and opened the same
// conversation again.
//
// The claim is that there is one conversation and not two: the editor kept an
// id, handed it back, and everything said on both sides of the closing is one
// lane in the record. So this driver counts the sessions the page lists for
// the profile, checks the one it finds is the lane the editor named, and reads
// that lane's turns back through the product's own door.

import {launchBrowser, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, profile, route, ...wanted] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!url || !profile || !route || wanted.length === 0) {
  console.error("usage: editor_door_resume_driver.mjs <url> <profile> <route> <said>...");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "editor-resume"});

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

// One conversation. Before the editor could hand an id back, each launch
// opened a lane of its own and this list grew a row per launch.
await b.waitFor("the session is in the list", async () =>
  b.evaluate(`document.querySelectorAll("#sessions .session-row").length > 0`), 300);
const listed = await b.evaluate(
  `JSON.stringify([...document.querySelectorAll("#sessions .session-row")].map(row => row.dataset.routeId))`,
).then(JSON.parse);
if (listed.length !== 1) {
  cleanup(1, `the page lists ${listed.length} sessions, not one: ${JSON.stringify(listed)}`);
}
if (listed[0] !== route) {
  cleanup(1, `the page lists ${listed[0]}, and the editor worked in ${route}`);
}
step(`the page lists one session, and it is the one the editor named`);

// And both turns are on it: what was said before the editor closed, and what
// was said after it came back.
const history = await b.evaluate(`(async () => {
  const answer = await fetch("/api/routes/" + encodeURIComponent(${JSON.stringify(route)}) + "/history?limit=200");
  const history = await answer.json();
  return JSON.stringify((history.events || [])
    .filter((event) => event.kind === "host/turn_written")
    .map((event) => ({text: event.payload?.text || "", surface: event.payload?.correspondent?.surface})));
})()`).then(JSON.parse);
for (const said of wanted) {
  if (!history.some((turn) => turn.text.includes(said))) {
    cleanup(1, `the lane does not carry "${said}": ${JSON.stringify(history)}`);
  }
}
if (!history.every((turn) => turn.surface === "editor")) {
  cleanup(1, `a turn on this lane was not written from an editor: ${JSON.stringify(history)}`);
}
step(`both turns are on one lane (${history.length} of them, all from an editor)`);

console.log("editor resume OK");
console.log(JSON.stringify({sessions: listed.length, route, turns: history.length}));
cleanup(0, "editor resume done");
