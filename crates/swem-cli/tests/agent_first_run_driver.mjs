// A person who has nothing yet: no project, no agent, no profile.
//
// They open the product, land in the Agent space, and the only thing there is
// a list of agents this computer does not have. They install one from the
// exact plan they are shown, answering the consent question themselves, and
// the agent becomes theirs to use - with no restart of the product.
//
// Every step is a real click or a real dialog on the running product.

import {launchBrowser, sleep, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, agentId] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!url || !agentId) {
  console.error("usage: agent_first_run_driver.mjs <url> <agent-id>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "agent-first-run"});

step("the product opens");
await b.waitFor("the shell", async () => b.exists("#space-agent"));

// The harness alone: the Agent space and the Store, and no space of a
// server nobody declared. An agent is had from here, on a first run, with
// nothing else installed.
if (await b.exists("#space-project")) cleanup(1, "the harness offers a Project space of its own");
step("the harness draws Agent and Store, and nothing of a server's");

await b.click("#space-agent");
await b.waitFor("the first-run list of agents", async () => b.exists("#agent-options .agent-option"));

const before = await b.evaluate(`(() => {
  const row = document.querySelector('[data-agent-id=${JSON.stringify(agentId)}]');
  return row ? row.textContent : "missing";
})()`);
if (before !== "Install") {
  cleanup(1, `the agent ${agentId} is not offered for installation, the button says ${JSON.stringify(before)}`);
}
step(`${agentId} is offered, not installed`);

step("install");
await b.click(`[data-agent-id=${JSON.stringify(agentId)}]`);

// The consent question is a native dialog. cdp_browser answers it and keeps
// what it said, so the walk can check the person was told what is fetched.
await b.waitFor("the consent question", async () => b.dialogs.length > 0);
const question = b.dialogs[0].message;
if (!/Install/.test(question) || !/registry|described by/i.test(question)) {
  cleanup(1, `the consent question does not say what is being installed: ${JSON.stringify(question)}`);
}
step("consent given to a named distribution");

// The install runs, and the product then creates the local profile itself.
// The install fetches and unpacks, then the product makes the profile
// itself. Either it ends with a profile or it says why not; the walk waits
// for whichever comes, so a refusal is read rather than timed out on.
await b.waitFor(
  "the install to finish",
  async () => b.evaluate(`(() => {
    const status = (document.getElementById("onboarding-status")||{textContent:""}).textContent;
    const working = /Reading|Installing|Creating/.test(status);
    return !working && (status.length > 0 || document.querySelectorAll("#profiles option").length > 0);
  })()`),
  300,
  1000,
);

const status = await b.evaluate(`(document.getElementById("onboarding-status")||{textContent:""}).textContent`);
if (status) cleanup(1, `the product refused after installing: ${status}`);
step("a profile exists");

// Usable, not merely listed: the product offers to start a session with it.
await b.waitFor("the product offers to start a session", async () =>
  b.evaluate(`document.querySelector("#open-new")?.disabled === false`));
step("the agent can be started");

// Having got an agent, the person decides how it asks for permission. The
// choice is a control with the host's own words under it, not JSON in a box,
// and what they pick is still there after a reload.
step("choose how it asks");
await b.click('[data-agent-tab="environment"]');
await b.waitFor("the permission choice", async () => b.exists("#profile-permissions option"));
const choices = await b.evaluate(
  `JSON.stringify([...document.querySelectorAll("#profile-permissions option")].map(o => o.value))`,
).then(JSON.parse);
const alone = choices.find((value) => value !== "surface-permissions");
if (!alone) cleanup(1, `the only choice offered is the one it already had: ${JSON.stringify(choices)}`);
await b.evaluate(`(() => {
  const field = document.getElementById("profile-permissions");
  Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value').set.call(field, ${JSON.stringify(alone)});
  field.dispatchEvent(new Event('change', {bubbles: true}));
})()`);
await b.waitFor("the choice is saved", async () =>
  b.evaluate(`document.getElementById("profile-permissions")?.value === ${JSON.stringify(alone)}
    && (document.getElementById("environment-status")||{textContent:""}).textContent === "Saved."`), 300);
const boundary = await b.evaluate(
  `(document.getElementById("permissions-summary")||{textContent:""}).textContent`);
if (!boundary || boundary.length < 20) {
  cleanup(1, `the choice says nothing about the boundary it sets: ${JSON.stringify(boundary)}`);
}
step("the boundary is stated in the page's own words");

await b.click('[data-agent-tab="conversation"]');

// And what all of it was for: a conversation. Still with no project in
// existence, the person starts a session with the agent they just installed
// and says something, and the agent's own answer comes back.
step("start a session");
await b.click("#open-new");
await b.waitFor("the session opens", async () =>
  b.evaluate(`(document.getElementById("route")||{textContent:"-"}).textContent !== "-"
    && (document.getElementById("connection-id")||{textContent:"-"}).textContent !== "-"`), 300);

const said = "hello from a person who has no project";
await b.evaluate(`(() => {
  const field = document.getElementById("prompt-text");
  Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set.call(field, ${JSON.stringify(said)});
  field.dispatchEvent(new Event('input', {bubbles: true}));
})()`);
await b.click("#send");
await b.waitFor("the turn ends", async () =>
  b.evaluate(`(document.getElementById("turn-outcome")||{textContent:""}).textContent.startsWith("turn:")`), 300);

const conversation = await b.evaluate(
  `JSON.stringify([...document.querySelectorAll("#conversation .message .message-body")].map(node => node.textContent))`,
).then(JSON.parse);
if (!conversation.some((text) => text.includes(said))) {
  cleanup(1, `what the person said is not in the conversation: ${JSON.stringify(conversation)}`);
}
if (conversation.length < 2) {
  cleanup(1, `the agent said nothing back: ${JSON.stringify(conversation)}`);
}
const outcome = await b.evaluate(`(document.getElementById("turn-outcome")||{textContent:""}).textContent`);
step(`the agent answered (${outcome})`);

// The rail names the conversation they just had. It reads the record, and the
// record is written by the turn, so a card still saying "Nothing said yet"
// beside a conversation on the screen is the page telling them about a
// session that no longer exists.
await b.waitFor("the rail names what was said", async () =>
  b.evaluate(`[...document.querySelectorAll("#sessions .session-row .session-name")]
    .some((row) => row.textContent && row.textContent !== "Nothing said yet")`), 100);
const named = await b.evaluate(
  `document.querySelector("#sessions .session-row .session-name")?.textContent ?? ""`);
step(`the rail calls it: ${named}`);

// The rest of what a harness is for, still with no project anywhere: a
// terminal in the agent's own environment, and a server of the person's own
// that the agent attaches. Neither is a project, and neither should need one.
step("a terminal in the agent's environment");
await b.click('[data-agent-tab="terminal"]');
await b.waitFor("the terminal panel", async () => b.exists('[data-agent-panel="terminal"]:not([hidden])'));
await b.click("#terminal-open");
await b.waitFor("a terminal running", async () =>
  b.evaluate(`(document.getElementById("terminal-id")||{textContent:""}).textContent.length > 0`), 200);
const screenText = () => b.evaluate(
  `[...document.querySelectorAll("#terminal-screen .xterm-rows div")].map((row) => row.textContent).join("\\n")`);
await b.waitFor("a prompt", async () => (await screenText()).trim().length > 0, 200);
await b.click("#terminal-screen");
await b.send("Input.insertText", {text: "echo swem-no-project"});
await b.send("Input.dispatchKeyEvent", {type: "keyDown", windowsVirtualKeyCode: 13, key: "Enter", text: "\r"});
await b.send("Input.dispatchKeyEvent", {type: "keyUp", windowsVirtualKeyCode: 13, key: "Enter"});
await b.waitFor("the command answers", async () => {
  const text = await screenText();
  // Once on the line that was typed, once as the shell's answer.
  return text.split("swem-no-project").length > 2;
}, 150);
const terminalId = await b.evaluate(`(document.getElementById("terminal-id")||{textContent:""}).textContent`);
step("a command ran in it and answered");

step("a server of the person's own");
await b.click('[data-agent-tab="environment"]');
await b.waitFor("the environment panel", async () => b.exists('[data-agent-panel="environment"]:not([hidden])'));
await b.click("#declare-server-open");
await b.waitFor("the declaration form", async () => b.exists("#declare-server"));
await b.evaluate(`(() => {
  document.getElementById("server-name").value = "notes";
  document.getElementById("server-command").value = "/bin/cat";
  document.getElementById("server-args").value = "";
})()`);
await b.click("#declare-server-save");
await b.waitFor("the declared server listed", async () =>
  b.exists('#mcp-servers .server-row[data-server="notes"][data-origin="catalogue"]'));
await b.click('.server-attach[data-server="notes"]');
await b.waitFor("the person's own server attached", async () =>
  b.evaluate(`!!document.querySelector('.server-attach[data-server="notes"]')?.checked`));
step("declared and attached, with no project in existence");

// One installed agent is not one person. A second person adds their own
// profile of the same agent from the rail, names it, and gets their own
// working directory and their own sessions - without installing anything
// again and without touching the first person's conversation.
step("a second person's profile of the same agent");
await b.click('[data-agent-tab="conversation"]');
await b.click("#add-profile-open");
await b.waitFor("the form for a second profile", async () => b.exists("#new-profile-id"));
await b.evaluate(`(() => {
  const field = document.getElementById("new-profile-id");
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(field, "ada");
  field.dispatchEvent(new Event('input', {bubbles: true}));
})()`);
// Which agent this profile is of. The form has always offered the choice and
// no walk had ever made it, so nothing said the chosen one is the one the
// profile ends up being. Where this machine has more than one agent the
// choice is a different one than the form opened on, which is the only way
// to tell a working chooser from one that is ignored.
const agentChoice = await b.evaluate(`(() => {
  const field = document.getElementById("new-profile-agent");
  const offered = [...field.options].map((option) => option.value);
  const other = offered.find((value) => value !== field.value) ?? field.value;
  Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value').set.call(field, other);
  field.dispatchEvent(new Event('change', {bubbles: true}));
  return JSON.stringify({offered, chosen: other, opened_on: offered[0] ?? ""});
})()`).then(JSON.parse);
if (agentChoice.offered.length === 0) {
  cleanup(1, "the form offers no agent to make a profile of");
}
await b.click("#add-profile");
await b.waitFor("the second profile listed", async () =>
  b.evaluate(`[...document.querySelectorAll("#profiles option")].some(o => o.value === "ada")`), 300);
const selected = await b.evaluate(`document.getElementById("profiles")?.value`);
if (selected !== "ada") cleanup(1, `adding a profile did not select it, the rail shows ${JSON.stringify(selected)}`);
// And it is a profile of the agent that was chosen, read from the product
// rather than from the form that asked.
const madeOf = await b.evaluate(`(async () => {
  const profiles = await (await fetch("/api/profiles")).json();
  const mine = profiles.find((profile) => profile.profile_id === "ada");
  return JSON.stringify({agent: mine?.agent_id ?? "", chosen: ${JSON.stringify(agentChoice.chosen)}});
})()`).then(JSON.parse);
if (madeOf.agent !== madeOf.chosen) {
  cleanup(1, `the profile was made of another agent than the one chosen: ${JSON.stringify(madeOf)}`);
}
step(`made of the agent chosen: ${madeOf.chosen}, one of ${agentChoice.offered.length} offered`);
// The proof that it is a second person and not a second name for the same
// one: the first profile held the conversation above, so it has a session;
// this one starts with none.
await b.waitFor("the new profile has its own, empty session list", async () =>
  b.exists("#sessions-empty"), 300);
const mine = await b.evaluate(
  `JSON.stringify([...document.querySelectorAll("#sessions .session-row")].map(row => row.dataset.routeId))`,
).then(JSON.parse);
if (mine.length !== 0) cleanup(1, `the second profile inherited sessions: ${JSON.stringify(mine)}`);
step("it has its own, empty session list");

// Where this profile's agent runs. The page reads it from the product's own
// list of environments rather than naming one, so what is shown here is what
// the product would actually use.
await b.click('[data-agent-tab="environment"]');
await b.waitFor("the environment panel", async () => b.exists('[data-agent-panel="environment"]:not([hidden])'));
await b.waitFor("the page says where the agent runs", async () =>
  b.evaluate(`(document.getElementById("profile-environment")?.textContent || "").length > 0`), 200);
const runsIn = await b.evaluate(`JSON.stringify({
  named: document.getElementById("profile-environment")?.dataset.environment || "",
  shown: document.getElementById("profile-environment")?.textContent || "",
})`).then(JSON.parse);
if (!runsIn.named) cleanup(1, `the page does not say where the agent runs: ${JSON.stringify(runsIn)}`);
step(`it runs ${runsIn.shown} (${runsIn.named})`);

const report = await b.evaluate(`JSON.stringify({
  profiles: [...document.querySelectorAll("#profiles option")].map(option => option.value),
  runs_in: ${JSON.stringify(runsIn.named)},
  runs_in_shown: ${JSON.stringify(runsIn.shown)},
  terminal: ${JSON.stringify(terminalId)},
  consent: ${JSON.stringify(question)},
})`);
console.log("first run OK");
console.log(report);
console.log(JSON.stringify({outcome, conversation}));
cleanup(0, "agent first run done");
