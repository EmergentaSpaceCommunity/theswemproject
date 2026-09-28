// A person leaves a standing instruction, and the product carries it out
// without them.
//
// Everything else in this product happens because somebody asked for it in
// the moment. This walk sets a schedule, then touches nothing: no session is
// started by hand, no prompt is typed. What proves it ran is a file in the
// agent's outbox that only the agent could have written, and a record that
// says the clock wrote the turn rather than a person.

import {launchBrowser, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, answer] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!url || !answer) {
  console.error("usage: agent_schedule_driver.mjs <url> <answer>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "agent-schedule"});

step("the product opens");
const profile = await b.makeAgent("hands");
step("the agent is theirs");

// Nothing has run and nothing is scheduled.
await b.openAgent(profile, "settings");
await b.waitFor("the environment panel", async () => b.exists('[data-agent-panel="environment"]:not([hidden])'));
const before = await b.evaluate(`document.querySelectorAll("#schedules .schedule-row").length`);
if (before !== 0) cleanup(1, `a new profile already has ${before} schedules`);
step("nothing is scheduled yet");

// The standing instruction, in the words a person would type to the agent.
await b.fill("#schedule-id", "morning");
await b.fill("#schedule-say", JSON.stringify({write: "outbox/from-the-clock.txt", text: answer}));
await b.fill("#schedule-minutes", "60");
await b.click("#schedule-save");
await b.waitFor("the schedule is listed", async () =>
  b.exists('#schedules .schedule-row[data-schedule="morning"]'), 200);
step("a standing instruction is set");

// And now nothing. No session started, no prompt typed. The clock is the
// product's, so what happens next happens without this page.
step("waiting for the clock");
await b.openAgent(profile, "files");
await b.waitFor("the Files panel", async () => b.exists('[data-agent-panel="files"]:not([hidden])'));
await b.waitFor(
  "the clock to run it",
  async () => {
    // The clock runs behind the page, so the page is asked rather than told.
    await b.click("#files-refresh");
    return b.exists('[data-files-area="outbox"] [data-file-name="from-the-clock.txt"]');
  },
  120,
  1000,
);
step("the agent did the work with nobody watching");

const href = await b.evaluate(
  `document.querySelector('[data-files-area="outbox"] [data-file-name="from-the-clock.txt"] a')?.getAttribute("href")`);
const served = await b.evaluate(`(async () => {
  const response = await fetch(${JSON.stringify(href)});
  return JSON.stringify({status: response.status, body: await response.text()});
})()`).then(JSON.parse);
if (served.body !== answer) cleanup(1, `the clock's turn produced something else: ${JSON.stringify(served)}`);
step("it wrote what the instruction said");

// The schedule now says when it ran, and the session it wrote into is in the
// list beside the ones a person started.
await b.openAgent(profile, "settings");
await b.waitFor("the schedule says it ran", async () => {
  await b.click("#schedules-refresh");
  return b.evaluate(`!/not said yet/i.test((document.querySelector('[data-schedule-outcome="morning"]')||{textContent:""}).textContent)`);
}, 60);
const outcome = await b.evaluate(
  `(document.querySelector('[data-schedule-outcome="morning"]')||{textContent:""}).textContent`);
step(`the schedule records its run (${outcome})`);

// The clock is a participant of the chat it speaks in: the chat says who said
// it, and it was not a person at a browser. Read through the product's own
// door, because that is where a person would read it.
const record = await b.evaluate(`(async () => {
  const schedules = await (await fetch("/api/profiles/hands/schedules")).json();
  // The whole standing instruction as the product holds it, read before it
  // is forgotten: what it claimed, and the chat it kept.
  const kept = schedules[0];
  const chat = await (await fetch("/api/chats/" + encodeURIComponent(kept?.chat_id))).json();
  return JSON.stringify({route: kept?.route_id, chat, kept});
})()`).then(JSON.parse);
const first = (record.chat.messages || [])[0];
const clock = (record.chat.chat?.members || []).find((member) => member.participant_id === first?.sender_id);
const wrote = {surface: first?.channel, author: clock?.name, kind: clock?.kind};
if (wrote.surface !== "schedule" || wrote.author !== "morning" || wrote.kind !== "schedule") {
  cleanup(1, `the record does not say the clock said it: ${JSON.stringify(record)}`);
}
step(`the chat says who said it (${wrote.author}, a ${wrote.kind})`);

// And stopped again. A standing instruction a person cannot stop is one the
// product goes on running without them, so Forget is the other half of
// setting one - and no walk had ever pressed it.
await b.click('.schedule-forget[data-schedule="morning"]');
await b.waitFor("the schedule leaves the list", async () =>
  !(await b.exists('#schedules .schedule-row[data-schedule="morning"]')), 200);
// Refreshing asks the host rather than the page, so what comes back is the
// host's answer and not a row React took away on its own.
await b.click("#schedules-refresh");
await b.waitFor("the host agrees it is gone", async () =>
  !(await b.exists('#schedules .schedule-row[data-schedule="morning"]')), 100);
const left = await b.evaluate(
  `(async () => JSON.stringify(await (await fetch("/api/profiles/hands/schedules")).json()))()`,
).then(JSON.parse);
if (left.length !== 0) cleanup(1, `the clock still holds the instruction: ${JSON.stringify(left)}`);
// What it already did is the person's work, and stays where it was put.
await b.openAgent(profile, "files");
await b.waitFor("the Files panel", async () => b.exists('[data-agent-panel="files"]:not([hidden])'));
await b.click("#files-refresh");
await b.waitFor("what the clock wrote is still there", async () =>
  b.exists('[data-files-area="outbox"] [data-file-name="from-the-clock.txt"]'), 100);
step("the standing instruction stops when a person stops it, and what it did stays");

console.log("schedule OK");
console.log(JSON.stringify({outcome, served, route: record.route, wrote, schedule: record.kept}));
cleanup(0, "agent schedule done");
