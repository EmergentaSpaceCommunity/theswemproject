// A person's agent runs a command, and the person watches it.
//
// The agent has no shell of its own here: it asks the client to run the
// command, which is what ACP's `terminal/*` are for. The client is this
// product, so the command runs in the profile's own environment - and the
// terminal it runs in is one of the person's, listed in the same panel they
// open their own in. Watching it is a click.

import {launchBrowser, sleep, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, marker] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!url || !marker) {
  console.error("usage: agent_terminal_driver.mjs <url> <marker>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "agent-terminal"});

const setValue = async (id, value, kind = "HTMLInputElement") =>
  b.evaluate(`(() => {
    const field = document.getElementById(${JSON.stringify(id)});
    Object.getOwnPropertyDescriptor(window.${kind}.prototype, 'value').set.call(field, ${JSON.stringify(value)});
    field.dispatchEvent(new Event('input', {bubbles: true}));
  })()`);

// `until` is what this turn is being asked for: the outcome and the messages
// it produced arrive on different lanes, so a read taken the moment the turn
// ends can find a conversation the page has not projected yet.
const say = async (text, until = (messages) => messages.length > 0) => {
  await setValue("prompt-text", text, "HTMLTextAreaElement");
  await b.click("#send");
  await b.waitFor("the turn ends", async () =>
    b.evaluate(`(document.getElementById("turn-outcome")||{textContent:""}).textContent.startsWith("turn:")`), 300);
  return b.bodiesUntil(until);
};

const screenText = async () =>
  b.evaluate(`[...document.querySelectorAll("#terminal-screen .xterm-rows div")].map((row) => row.textContent).join("\\n")`);

step("the product opens");
await b.waitFor("the shell", async () => b.exists("#space-agent"));
await b.click("#space-agent");

await b.waitFor("the agent offered", async () => b.exists("#agent-options .agent-option"));
await b.click('[data-agent-id="hands"]');
await b.waitFor("a profile exists", async () =>
  b.evaluate(`document.querySelectorAll("#profiles option").length > 0`), 200);
step("the agent is theirs");

// Whether this agent may run a command on its own is the person's choice,
// and it is the same one as whether it may edit a file on its own. Made
// before the session opens, because that is when it is read.
await b.click('[data-agent-tab="environment"]');
await b.waitFor("the environment panel", async () => b.exists('[data-agent-panel="environment"]:not([hidden])'));
await b.waitFor("the permission choices", async () =>
  b.evaluate(`document.querySelectorAll("#profile-permissions option").length > 1`), 300);
// The select is React's, so what it reads back a moment after the change is
// still the profile's old value: the amendment is what moves it, and the wait
// below is what it means for the choice to have been made.
const chosen = await b.evaluate(`(() => {
  const field = document.getElementById("profile-permissions");
  if (!field) return "";
  const offered = [...field.options].map((option) => option.value);
  if (!offered.includes("workspace-permissions")) return offered.join(",");
  Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value').set.call(field, "workspace-permissions");
  field.dispatchEvent(new Event('change', {bubbles: true}));
  return "workspace-permissions";
})()`);
if (chosen !== "workspace-permissions") cleanup(2, `the page offers no working-alone choice: ${chosen}`);
await b.waitFor("the choice saved", async () =>
  b.evaluate(`document.getElementById("profile-permissions")?.value === "workspace-permissions"`), 300);
step("the person lets it work alone inside its workspace");

await b.click("#open-new");
await b.waitFor("the session opens", async () =>
  b.evaluate(`(document.getElementById("route")||{textContent:"-"}).textContent !== "-"`), 300);
step("a session is open");

await b.click('[data-agent-tab="conversation"]');
await b.waitFor("the composer", async () => b.exists("#prompt-text"));

// The command itself: it writes a file where the agent works and says where
// that is, so both halves of "in the profile's environment" are checked.
const ran = await say(
  JSON.stringify({run: `echo ${marker} > ran.txt; pwd`}),
  (messages) => messages.some((text) => text.startsWith("ran ")),
);
if (!ran.some((text) => text.startsWith("ran ") && text.includes("/"))) {
  cleanup(1, `the agent could not run a command: ${JSON.stringify(ran)}`);
}
step("the agent ran a command and read what it said");

// And now the part a person can see: a command left running, found in their
// own terminal panel and watched from there.
const kept = await say(
  JSON.stringify({run: `echo ${marker}-watched; sleep 20`, keep: true}),
  (messages) => messages.some((text) => text.includes("running ")),
);
if (!kept.some((text) => text.includes("running "))) {
  cleanup(1, `the agent could not leave a command running: ${JSON.stringify(kept)}`);
}
await b.click('[data-agent-tab="terminal"]');
await b.waitFor("the terminal panel", async () => b.exists('[data-agent-panel="terminal"]:not([hidden])'));
await b.waitFor("the agent's terminal is listed", async () =>
  b.exists('.terminal-list li[data-opened-by="agent"]'), 300);
const said = await b.evaluate(
  `document.querySelector('.terminal-list li[data-opened-by="agent"] .terminal-what')?.textContent ?? ""`);
if (!said.startsWith("Your agent") || !said.includes(`${marker}-watched`)) {
  cleanup(1, `the panel does not say whose terminal it is or what it runs: ${JSON.stringify(said)}`);
}
step(`the panel lists it: ${said}`);

await b.click('.terminal-list li[data-opened-by="agent"] .terminal-watch');
await b.waitFor("what the agent's command said", async () =>
  (await screenText()).includes(`${marker}-watched`), 200);
step("the person watched their agent's command");

const terminalId = await b.evaluate(
  `document.querySelector('.terminal-list li[data-opened-by="agent"]')?.getAttribute("data-terminal-id") ?? ""`);

// Watching is looking, not owning: a person can step away from their agent's
// command, and open one of their own, without ending what the agent is doing.
await b.click("#terminal-detach");
await b.waitFor("the panel lets go", async () => !(await b.exists("#terminal-detach")), 100);
const canOpen = await b.evaluate(
  `document.getElementById("terminal-open")?.disabled === false`);
if (!canOpen) cleanup(1, "a person watching their agent cannot open a terminal of their own");
const still = await b.evaluate(
  `document.querySelectorAll('.terminal-list li[data-opened-by="agent"]').length`);
if (still !== 1) {
  cleanup(1, `watching cost the agent its terminal: ${still} left`);
}
step("the person stepped away, the agent's command kept running, and their own terminal is theirs to open");


// --- The other mode: every command is put to the person --------------------
//
// This is the mode a person picks when they want to be asked, so a command
// has to reach them as a question with the command line in it, and run only
// on a yes.
await b.click('[data-agent-tab="environment"]');
await b.waitFor("the environment panel", async () => b.exists('[data-agent-panel="environment"]:not([hidden])'));
await b.evaluate(`(() => {
  const field = document.getElementById("profile-permissions");
  Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value').set.call(field, "surface-permissions");
  field.dispatchEvent(new Event('change', {bubbles: true}));
})()`);
await b.waitFor("the choice saved", async () =>
  b.evaluate(`document.getElementById("profile-permissions")?.value === "surface-permissions"`), 300);
await b.click("#open-new");
await b.waitFor("a second session opens", async () =>
  b.evaluate(`(document.getElementById("route")||{textContent:"-"}).textContent !== "-"`), 300);
await b.click('[data-agent-tab="conversation"]');
await b.waitFor("the composer", async () => b.exists("#prompt-text"));

// Ask, wait for the question, answer it. `say` cannot be used here: the turn
// does not end until somebody answers.
const askAndAnswer = async (text, optionId) => {
  await setValue("prompt-text", text, "HTMLTextAreaElement");
  await b.click("#send");
  await b.waitFor("the question", async () =>
    b.evaluate(`(document.getElementById("permission")||{style:{}}).style.display === "grid"`), 200);
  const question = await b.textOf("permission-title");
  // Whose words the person is reading. An agent's report and the host's own
  // statement carry the same kind of sentence and do not mean the same
  // thing, so the dialog has to say which one this is.
  const whose = await b.textOf("permission-whose");
  const provenance = await b.evaluate(
    `(document.getElementById("permission")||{dataset:{}}).dataset.provenance || ""`,
  );
  await b.click(`.permission-option[data-option-id=${JSON.stringify(optionId)}]`);
  await b.waitFor("the turn ends", async () =>
    b.evaluate(`(document.getElementById("turn-outcome")||{textContent:""}).textContent.startsWith("turn:")`), 300);
  const said = await b.bodiesUntil((messages) => messages.length > 0);
  return {question, said, whose, provenance};
};

const no = await askAndAnswer(JSON.stringify({run: `echo ${marker} > refused.txt`}), "do-not-run");
if (!no.question.includes(`echo ${marker} > refused.txt`)) {
  cleanup(1, `the question does not say what would run: ${JSON.stringify(no.question)}`);
}
if (no.provenance !== "host_callback" || !no.whose.includes("runs here if you allow it")) {
  cleanup(1, `the question does not say whose words these are: ${JSON.stringify([no.provenance, no.whose])}`);
}
if (!no.said.some((text) => text.includes("not allowed to run"))) {
  cleanup(1, `a command nobody allowed was not refused: ${JSON.stringify(no.said)}`);
}
step(`the person was asked and said no: ${no.question}`);

const yes = await askAndAnswer(JSON.stringify({run: `echo ${marker} > allowed.txt`}), "run-once");
if (!yes.said.some((text) => text.startsWith("ran "))) {
  cleanup(1, `the allowed command did not run: ${JSON.stringify(yes.said)}`);
}
step("the person said yes and it ran");

await sleep(200);
console.log("agent terminal OK");
console.log(JSON.stringify({terminalId, said}));
cleanup(0, "agent terminal done");
