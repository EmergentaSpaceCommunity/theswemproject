// A person makes a schedule, and the product says it on time without them.
//
// Everything else in this product happens because somebody asked for it in
// the moment. This walk makes a schedule in the agent's Schedules, then
// touches nothing: no chat is begun by hand, nothing is typed to the agent.
// What proves it was said is a file in the agent's outbox that only the
// agent could have written, a run that says it was answered, and a chat
// whose first message is the schedule's own.

import {launchBrowser, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, answer] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!url || !answer) {
  console.error("usage: agent_schedule_driver.mjs <url> <answer>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "agent-schedule"});
const panel = '[data-agent-panel="schedules"]:not([hidden])';
const choose = (selector, value) => b.evaluate(`(() => {
  const field = document.querySelector(${JSON.stringify(selector)});
  Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set.call(field, ${JSON.stringify(value)});
  field.dispatchEvent(new Event("change", {bubbles: true}));
})()`);
const shown = () => b.evaluate(`document.querySelector(${JSON.stringify(panel)})?.innerText ?? ""`);

step("the product opens");
const profile = await b.makeAgent("hands");
const agent = await b.agentOf(profile);
const kept = async () => (await b.ask(`/api/schedules?agent=${agent}`)).body;
step("the agent is theirs");

await b.openAgent(profile, "schedules");
await b.waitFor("its schedules", async () => (await shown()).includes("on time yet"), 100);
if ((await kept()).schedules.length !== 0) cleanup(1, "a new agent is told something on time already");
step("nothing is said to it on time yet");

// What it is told, in the words a person would write to it; every minute,
// so the walk need not wait long; in a chat of its own.
const told = JSON.stringify({write: "outbox/from-the-clock.txt", text: answer});
if (!(await b.pressText(`${panel} button`, "New schedule"))) cleanup(1, "a schedule cannot be made");
await b.waitFor("the form", async () => b.exists(".w-dialog textarea"));
await b.fill(".w-dialog textarea", told);
await choose(".w-dialog select", "every");
await b.waitFor("how often", async () => b.exists('.w-dialog input[type="number"]'));
await b.fill('.w-dialog input[type="number"]', "1");
if (!(await b.pressText(".w-dialog button", "Make the schedule"))) cleanup(1, "the form cannot be sent");
await b.waitFor("the schedule in the list", async () => (await shown()).includes("Every minute"), 100);
const made = (await kept()).schedules[0];
const owner = (await b.ask("/api/people")).body.owner;
if (made?.made_by !== owner.participant_id || !(await shown()).includes(owner.name)) {
  cleanup(1, `the schedule does not say who made it: ${JSON.stringify(made)}`);
}
step(`a schedule is made: ${made.when_in_words}, by ${made.made_by_name}`);

// And now nothing. The clock is the product's, so what happens next happens
// without this page.
step("waiting for its time");
await b.openAgent(profile, "files");
await b.waitFor("the Files panel", async () => b.exists('[data-agent-panel="files"]:not([hidden])'));
await b.waitFor(
  "its time to come",
  async () => {
    await b.click("#files-refresh");
    return b.exists('[data-files-area="outbox"] [data-file-name="from-the-clock.txt"]');
  },
  150,
  1000,
);
step("the agent did the work with nobody watching");

const href = await b.evaluate(
  `document.querySelector('[data-files-area="outbox"] [data-file-name="from-the-clock.txt"] a')?.getAttribute("href")`);
const served = await b.evaluate(`(async () => {
  const response = await fetch(${JSON.stringify(href)});
  return JSON.stringify({status: response.status, body: await response.text()});
})()`).then(JSON.parse);
if (served.body !== answer) cleanup(1, `what was said on time produced something else: ${JSON.stringify(served)}`);
step("it wrote what it was told to");

// The run is read where the schedule is, and says how it ended.
await b.openAgent(profile, "schedules");
await b.waitFor("the run, answered", async () => (await shown()).includes("Answered"), 120, 500);
const run = (await kept()).runs.at(-1);
step(`the run is kept: ${run.state}`);

// The schedule is a participant of the chat it speaks in: the chat says who
// said it, and it was not a person at a browser.
const chat = (await b.ask(`/api/chats/${run.chat_id}`)).body;
const first = (chat.messages ?? [])[0];
const clock = (chat.chat?.members ?? []).find((member) => member.participant_id === first?.sender_id);
const wrote = {channel: first?.channel, name: clock?.name, kind: clock?.kind, made_by: clock?.made_by};
if (wrote.channel !== "schedule" || wrote.kind !== "schedule" || wrote.made_by !== owner.participant_id) {
  cleanup(1, `the chat does not say the schedule said it: ${JSON.stringify(wrote)}`);
}
step(`the chat says who said it (a ${wrote.kind})`);

// Paused: the switch is off, and the host keeps it as not due.
await b.click(`${panel} button[role="switch"]`);
await b.waitFor("the schedule paused", async () =>
  (await b.evaluate(`document.querySelector(${JSON.stringify(`${panel} button[role="switch"]`)})?.getAttribute("aria-checked")`)) === "false"
    && (await shown()).includes("paused"), 100);
const paused = (await kept()).schedules[0];
if (paused.enabled || paused.next_due_ms) cleanup(1, `a paused schedule is still due: ${JSON.stringify(paused)}`);
step("the schedule is paused with its switch");

// And forgotten. What it already did is the person's work and stays.
if (!(await b.pressText(`${panel} button`, "Forget"))) cleanup(1, "a schedule cannot be forgotten");
await b.waitFor("the schedule gone", async () => (await shown()).includes("on time yet"), 100);
if ((await kept()).schedules.length !== 0) cleanup(1, "the host still keeps the schedule");
await b.openAgent(profile, "files");
await b.waitFor("the Files panel", async () => b.exists('[data-agent-panel="files"]:not([hidden])'));
await b.click("#files-refresh");
await b.waitFor("what was written is still there", async () =>
  b.exists('[data-files-area="outbox"] [data-file-name="from-the-clock.txt"]'), 100);
step("the schedule stops when a person stops it, and what it did stays");

console.log("schedule OK");
console.log(JSON.stringify({served, wrote, schedule: made, run}));
cleanup(0, "agent schedule done");
