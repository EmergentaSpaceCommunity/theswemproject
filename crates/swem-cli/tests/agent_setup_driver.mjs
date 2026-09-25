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

const said = () => b.evaluate(
  `JSON.stringify([...document.querySelectorAll("#conversation .message .message-body")].map(node => node.textContent))`,
).then(JSON.parse);
const say = async (text) => {
  const before = (await said()).length;
  await setValue("#prompt-text", text);
  await b.click("#send");
  await b.waitFor("the turn ends", async () =>
    b.evaluate(`(() => {
      const outcome = (document.getElementById("turn-outcome")||{textContent:""}).textContent;
      const messages = document.querySelectorAll("#conversation .message .message-body").length;
      return outcome.startsWith("turn:") && messages >= ${before} + 2;
    })()`), 300);
  return said();
};

step("the product opens");
await b.waitFor("the shell", async () => b.exists("#space-agent"));
await b.click("#space-agent");
await b.waitFor("the agent offered", async () => b.exists("#agent-options .agent-option"));
await b.click('[data-agent-id="hands"]');
await b.waitFor("a profile exists", async () =>
  b.evaluate(`document.querySelectorAll("#profiles option").length > 0`), 200);
step("the agent is theirs");

// --- Setup: a provider, the model, the role ------------------------------
await b.click('[data-agent-tab="environment"]');
await b.waitFor("the setup panel", async () => b.exists('[data-agent-panel="environment"]:not([hidden])'));
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

// --- The agent reads it back ------------------------------------------------
await b.click('[data-agent-tab="conversation"]');
await b.click("#open-new");
await b.waitFor("the session opens", async () =>
  b.evaluate(`(document.getElementById("route")||{textContent:"-"}).textContent !== "-"`), 300);
step("a session is open");
const answered = await say(JSON.stringify({read: "AGENTS.md"}));
const instructions = answered[answered.length - 1] ?? "";
if (!instructions.includes(role)) cleanup(1, `the agent did not get its role: ${instructions}`);
if (!instructions.includes("<!-- swem:system -->")) cleanup(1, `the role is not in SWEM's span: ${instructions}`);
step("the agent read its role from its own directory");

// The rail's card says what this profile asks for.
const chip = await b.evaluate(`document.querySelector("#profile-card .k-chip")?.textContent ?? ""`);
if (chip !== "quality") cleanup(1, `the profile card does not name the model: ${JSON.stringify(chip)}`);

console.log(JSON.stringify({model: await valueOf("#session-model"), chip, instructions_length: instructions.length}));
console.log("setup OK");
await sleep(200);
await b.close();
process.exit(0);
