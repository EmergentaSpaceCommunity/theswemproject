// A person gives their own agent a role and a model, and the agent reads
// the role back from where it lives.
//
// The agent is the fixture declared on this machine (`hands`), which reads
// files under its working directory when asked to. The role is written there
// as AGENTS.md before the session starts - the layout a declared agent gets -
// so `{"read":"AGENTS.md"}` answers with the role the person wrote. The
// model is asked for through a provider the person declares first.

import {launchBrowser, sleep, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, role] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!url || !role) {
  console.error("usage: agent_setup_driver.mjs <url> <role>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "agent-setup"});
/// What the walk gives the provider. Not a key of anything.
const KEY = "sk-walk-not-a-key-of-anything";

const setValue = async (selector, value) =>
  b.evaluate(`(() => {
    const field = document.querySelector(${JSON.stringify(selector)});
    const proto = field instanceof HTMLTextAreaElement ? HTMLTextAreaElement : HTMLInputElement;
    Object.getOwnPropertyDescriptor(proto.prototype, 'value').set.call(field, ${JSON.stringify(value)});
    field.dispatchEvent(new Event('input', {bubbles: true}));
  })()`);
const choose = async (selector, value) =>
  b.evaluate(`(() => {
    const field = document.querySelector(${JSON.stringify(selector)});
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(field, ${JSON.stringify(value)});
    field.dispatchEvent(new Event('change', {bubbles: true}));
  })()`);
const valueOf = (selector) => b.evaluate(`document.querySelector(${JSON.stringify(selector)})?.value ?? ""`);
const saved = (what) => b.waitFor(`${what} saved`, async () => (await b.textOf("environment-status")) === "Saved.", 150);

step("the product opens");
const profile = await b.makeAgent("hands");
step("the agent is theirs");

// --- Providers: a place a model is served from, and what opens it --------
await b.goTo("#/providers/models");
await b.waitFor("the shipped providers listed", async () => b.exists('.provider-row[data-provider="anthropic"][data-origin="built_in"]'));
await b.click("#add-model-provider-open");
await b.waitFor("the provider form", async () => b.exists("#declare-model-provider"));
await setValue("#provider-id", "desk");
await setValue("#provider-name", "The box under the desk");
await setValue("#provider-base-url", "http://127.0.0.1:8000/v1");
await choose("#provider-key-type", "generic_env_var");
await setValue("#provider-models", "quality · Quality");
await b.click("#add-model-provider");
await b.waitFor("the provider listed", async () => b.exists('.provider-row[data-provider="desk"][data-origin="declared"]'));
step("a provider is declared");

// What opens it is given once, here, and never typed into an agent.
if (!(await b.pressText('.provider-row[data-provider="desk"] button', "Give"))) cleanup(1, "the provider cannot be given what opens it");
await b.waitFor("the dialog that takes it", async () => b.exists('.w-dialog input[type="password"]'));
await setValue('.w-dialog input[type="password"]', KEY);
await setValue('.w-dialog input.k-mono', "DESK_API_KEY");
if (!(await b.pressText(".w-dialog button", "Give it"))) cleanup(1, "what was typed cannot be given");
await b.waitFor("the provider says it was given", async () =>
  (await b.evaluate(`document.querySelector('.provider-row[data-provider="desk"]')?.innerText ?? ""`)).includes("Given"), 100);
if ((await b.evaluate("document.body.innerText")).includes(KEY)) cleanup(1, "the page shows what was given back");
step("the provider is given what opens it, once");

// --- Settings: the model, what the agent is for --------------------------
await b.openAgent(profile, "settings");
await b.waitFor("the settings", async () => b.exists('[data-agent-panel="environment"]:not([hidden])'));
if (!(await b.pressText("#settings-together .k-row button", "Change"))) cleanup(1, "the model cannot be changed");
await b.waitFor("the providers to choose from", async () =>
  (await b.evaluate(`[...(document.getElementById("profile-model-provider")?.options ?? [])].map((o) => o.value).join(",")`)).includes("desk"), 100);
await choose("#profile-model-provider", "desk");
await saved("the provider");
await b.waitFor("its models offered", async () =>
  (await b.evaluate(`[...(document.getElementById("profile-model")?.options ?? [])].map((o) => o.value).join(",")`)).includes("quality"));
await choose("#profile-model", "quality");
await saved("the model");
step("the model is chosen");

await setValue("#profile-role", role);
await b.click("#save-role");
await saved("the role");
step("the role is written");

// It works by itself inside its folder, so a command it runs is not asked
// about: the walk is about what it was handed.
await choose("#profile-permissions", "workspace-permissions");
await b.waitFor("how it asks, taken", async () =>
  (await b.evaluate(`document.querySelector("#settings-permissions #environment-status")?.textContent ?? ""`)) === "Saved.", 150);

// --- The agent reads it back ------------------------------------------------
await b.openAgent(profile);
const handed = (await b.say(JSON.stringify({run: `test -n "$DESK_API_KEY" && echo it-was-handed-over || echo it-has-nothing`}))).at(-1) ?? "";
if (!handed.includes("it-was-handed-over")) cleanup(1, `the agent was not handed what its provider was given: ${handed}`);
if (JSON.stringify(await b.said()).includes(KEY)) cleanup(1, "what the provider was given is in the chat");
step("the agent was handed what opens its provider, and nobody typed it into the agent");

const answered = await b.say(JSON.stringify({read: "AGENTS.md"}));
const instructions = answered[answered.length - 1] ?? "";
if (!instructions.includes(role)) cleanup(1, `the agent did not get its role: ${instructions}`);
if (!instructions.includes("<!-- swem:system -->")) cleanup(1, `the role is not in SWEM's span: ${instructions}`);
if (!instructions.includes("## Who is speaking")) cleanup(1, `the agent was not told how it is told who speaks: ${instructions}`);
step("the agent read its role from its own directory");

// The agent's header says what it stands on and which model it asks for.
const chip = await b.evaluate(`[...document.querySelectorAll(".w-head .k-chip")].map((one) => one.textContent).join("|")`);
if (!chip.split("|").includes("quality")) cleanup(1, `the agent's header does not name the model: ${JSON.stringify(chip)}`);

console.log(JSON.stringify({chip, instructions_length: instructions.length}));
console.log("setup OK");
await sleep(200);
await b.close();
process.exit(0);
