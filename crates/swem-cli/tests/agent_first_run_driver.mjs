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

// The harness alone: agents, the Store, and no App of a server nobody
// declared. An agent is had from here, on a first run, with nothing else
// installed.
await b.waitFor("the page asks for a first agent", async () => b.exists(".w-form"), 100);
const groups = await b.evaluate(`[...document.querySelectorAll('nav[aria-label="Workbench"] .k-eyebrow')].map((one) => one.textContent).join("|")`);
if (groups.split("|").includes("Apps")) cleanup(1, `the harness offers an App of its own: ${groups}`);
step("the harness draws agents and the Store, and nothing of a server's");

const offered = async () => (await b.ask("/api/onboarding")).body?.agents?.find((one) => one.agent_id === agentId);
const before = await offered();
if (!before || before.available) {
  cleanup(1, `the agent ${agentId} is not offered for installation: ${JSON.stringify(before)}`);
}
step(`${agentId} is offered, not installed`);

step("install");
const row = JSON.parse(await b.evaluate(`(() => {
  const label = [...document.querySelectorAll(".w-form label")].find((one) => one.innerText.includes(${JSON.stringify(before.name)}));
  const button = label?.querySelector("button");
  if (!button) return "null";
  button.scrollIntoView({block: "center"});
  const box = button.getBoundingClientRect();
  return JSON.stringify({x: box.left + box.width / 2, y: box.top + box.height / 2, says: button.innerText});
})()`));
if (!row || !/Install/.test(row.says)) cleanup(1, `the form offers no way to install ${before.name}: ${JSON.stringify(row)}`);
await b.clickAt(row.x, row.y);

// The consent question is a native dialog. cdp_browser answers it and keeps
// what it said, so the walk can check the person was told what is fetched.
await b.waitFor("the consent question", async () => b.dialogs.length > 0);
const question = b.dialogs[0].message;
if (!/Install/.test(question) || !/registry|described by/i.test(question)) {
  cleanup(1, `the consent question does not say what is being installed: ${JSON.stringify(question)}`);
}
step("consent given to a named distribution");

// The install fetches and unpacks, then the product makes the agent itself.
// Either it ends with an agent or it says why not; the walk waits for
// whichever comes, so a refusal is read rather than timed out on.
let refused = "";
await b.waitFor(
  "the install to finish",
  async () => {
    if ((await b.agentOf(agentId)) !== null) return true;
    refused = await b.evaluate(`document.querySelector(".w-form .k-notice")?.textContent ?? ""`);
    return refused.length > 0 && !/Reading|Installing|Creating/.test(refused);
  },
  300,
  1000,
);
if ((await b.agentOf(agentId)) === null) cleanup(1, `the product refused after installing: ${refused}`);
step("an agent exists");

// Having got an agent, the person decides how it asks for permission. The
// choice is a control with the host's own words under it, not JSON in a box,
// and what they pick is still there after a reload.
step("choose how it asks");
await b.openAgent(agentId, "settings");
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

// And what all of it was for: a conversation. Still with no project in
// existence, the person says something to the agent they just installed, and
// the agent's own answer comes back.
step("say something");
await b.openAgent(agentId);
const said = "hello from a person who has no project";
const conversation = await b.say(said);
if (!conversation.some((text) => text.includes(said))) {
  cleanup(1, `what the person said is not in the chat: ${JSON.stringify(conversation)}`);
}
if (conversation.length < 2) {
  cleanup(1, `the agent said nothing back: ${JSON.stringify(conversation)}`);
}
const outcome = "answered";
step("the agent answered");

// The list of chats names the one they just had, by what was said in it.
await b.waitFor("the list names what was said", async () =>
  b.evaluate(`[...document.querySelectorAll('aside[aria-label^="Chats with"] .k-rail-item')]
    .some((row) => row.innerText.includes(${JSON.stringify(said)}))`), 100);
const named = await b.evaluate(
  `document.querySelector('aside[aria-label^="Chats with"] .k-rail-item')?.innerText ?? ""`);
step(`the list calls it: ${named.split("\n")[0]}`);

// The rest of what a harness is for, still with no project anywhere: a
// terminal in the agent's own environment, and a server of the person's own
// that the agent attaches. Neither is a project, and neither should need one.
step("a terminal in the agent's environment");
await b.openAgent(agentId, "terminal");
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
await b.openAgent(agentId, "settings");
await b.waitFor("the environment panel", async () => b.exists('[data-agent-panel="environment"]:not([hidden])'));
await b.click("#declare-server-open");
await b.waitFor("the declaration form", async () => b.exists("#declare-server"));
await b.fill("#server-name", "notes");
await b.fill("#server-command", "/bin/cat");
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
step("a second agent on an engine this computer has");
// Which engine the second agent stands on is chosen in the form. Where this
// machine has more than one, the choice is another one than the first
// agent's, which is the only way to tell a working chooser from one that is
// ignored.
const engines = ((await b.ask("/api/onboarding")).body?.agents ?? []).filter((one) => one.available).map((one) => one.agent_id);
const agentChoice = {offered: engines, chosen: engines.find((one) => one !== agentId) ?? agentId};
if (agentChoice.offered.length === 0) {
  cleanup(1, "the form offers no engine to make an agent of");
}
await b.makeAgent(agentChoice.chosen, "ada");
const selected = await b.evaluate(`document.querySelector(".w-h1")?.textContent ?? ""`);
if (selected !== "ada") cleanup(1, `making an agent did not go to it, the page shows ${JSON.stringify(selected)}`);
// And it stands on the engine that was chosen, read from the product
// rather than from the form that asked.
const madeOf = await b.evaluate(`(async () => {
  const profiles = await (await fetch("/api/profiles")).json();
  const mine = profiles.find((profile) => profile.profile_id === "ada");
  return JSON.stringify({agent: mine?.agent_id ?? "", chosen: ${JSON.stringify(agentChoice.chosen)}});
})()`).then(JSON.parse);
if (madeOf.agent !== madeOf.chosen) {
  cleanup(1, `the agent stands on another engine than the one chosen: ${JSON.stringify(madeOf)}`);
}
step(`made of the engine chosen: ${madeOf.chosen}, one of ${agentChoice.offered.length} offered`);
// The proof that it is a second agent and not a second name for the same
// one: the first held the chat above; this one starts with none.
const ada = await b.agentOf("ada");
const mine = ((await b.ask("/api/chats")).body ?? []).filter((chat) => chat.members.some((member) => member.participant_id === ada));
if (mine.length !== 0) cleanup(1, `the second agent inherited chats: ${JSON.stringify(mine)}`);
await b.waitFor("the page offers to start its first chat", async () =>
  b.evaluate(`(document.querySelector(".k-empty")?.innerText ?? "").includes("Start a chat with ada")`), 100);
step("it has its own, empty list of chats");

// Where this profile's agent runs. The page reads it from the product's own
// list of environments rather than naming one, so what is shown here is what
// the product would actually use.
await b.openAgent("ada", "settings");
await b.waitFor("the environment panel", async () => b.exists('[data-agent-panel="environment"]:not([hidden])'));
await b.waitFor("the page says where the agent runs", async () =>
  b.evaluate(`(document.getElementById("profile-environment")?.textContent || "").length > 0`), 200);
const runsIn = await b.evaluate(`JSON.stringify({
  named: document.getElementById("profile-environment")?.dataset.environment || "",
  shown: document.getElementById("profile-environment")?.textContent || "",
})`).then(JSON.parse);
if (!runsIn.named) cleanup(1, `the page does not say where the agent runs: ${JSON.stringify(runsIn)}`);
step(`it runs ${runsIn.shown} (${runsIn.named})`);

const profiles = ((await b.ask("/api/profiles")).body ?? []).map((profile) => profile.profile_id);
const report = await b.evaluate(`JSON.stringify({
  profiles: ${JSON.stringify(profiles)},
  runs_in: ${JSON.stringify(runsIn.named)},
  runs_in_shown: ${JSON.stringify(runsIn.shown)},
  terminal: ${JSON.stringify(terminalId)},
  consent: ${JSON.stringify(question)},
})`);
console.log("first run OK");
console.log(report);
console.log(JSON.stringify({outcome, conversation}));
cleanup(0, "agent first run done");
