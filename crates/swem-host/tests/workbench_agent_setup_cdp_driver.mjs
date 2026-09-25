// A person gives their agent a model provider, a model, a role and a skill,
// and the agent gets them.
//
// usage: node <this-file> <workbench-url> [--again]
//
// Everything here is what a person does in Setup: declare where a model is
// served from, pick the provider and the model, write the role, add a skill;
// then start a session and see the model on the strip over the conversation,
// and read in the agent's own reply that the model reached it - by the
// variable at launch and by the session's own option. Then the page is
// reloaded and the session is found and resumed. `--again` is the second run
// of the product over the same directory: nothing is configured, everything
// configured before must still be there.
import {launchBrowser, sleep, cleanup} from "./cdp_browser.mjs";

const [url, mode] = process.argv.slice(2);
const again = mode === "--again";
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
const failureDir = process.env.SWEM_FAILURE_DIR;

const b = await launchBrowser({browser, url, label: "agent-setup", failureDir});
const step = (name) => console.log(`step: ${name}`);

/// Put a value into a field React controls, the way typing would.
const fill = async (selector, text) => {
  await b.evaluate(`(() => {
    const field = document.querySelector(${JSON.stringify(selector)});
    const proto = field instanceof HTMLTextAreaElement ? HTMLTextAreaElement : HTMLInputElement;
    Object.getOwnPropertyDescriptor(proto.prototype, "value").set.call(field, ${JSON.stringify(text)});
    field.dispatchEvent(new Event("input", {bubbles: true}));
  })()`);
};
/// Choose an option of a select React controls.
const choose = async (selector, value) => {
  await b.evaluate(`(() => {
    const field = document.querySelector(${JSON.stringify(selector)});
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set.call(field, ${JSON.stringify(value)});
    field.dispatchEvent(new Event("change", {bubbles: true}));
  })()`);
};
const valueOf = (selector) => b.evaluate(`document.querySelector(${JSON.stringify(selector)})?.value ?? ""`);
/// Wait for the setup status line to say the last change was kept.
const saved = async (what) => {
  await b.waitFor(`${what} saved`, async () => (await b.textOf("environment-status")) === "Saved.", 150);
};

await b.click("#space-agent");
await b.waitFor("the agent space", async () => b.exists('.app-shell[data-active-space="agent"]'));
await b.waitFor("an agent to set up", async () =>
  (await b.evaluate(`document.getElementById("profiles").options.length`)) >= 1);
await b.click('[data-agent-tab="environment"]');
await b.waitFor("the setup panel", async () => b.exists('[data-agent-panel="environment"]:not([hidden])'));

if (again) {
  // The second run of the product: what a person set up is still theirs.
  await b.waitFor("the provider still listed", async () => b.exists('.provider-row[data-provider="local"][data-origin="declared"]'));
  await b.waitFor("the model still chosen", async () => (await valueOf("#profile-model")) === "quality");
  const role = await valueOf("#profile-role");
  if (!role.includes("You are Ada.")) cleanup(1, `the role was forgotten: ${JSON.stringify(role)}`);
  await b.waitFor("the skill still there", async () => b.exists('.skill-row[data-skill="review"]'));
  console.log(JSON.stringify({again: true, model: await valueOf("#profile-model"), role}));
  await sleep(200);
  await b.close();
  process.exit(0);
}

// --- Where the model is served from ------------------------------------
step("a provider is declared");
await b.click("#add-model-provider-open");
await b.waitFor("the provider form", async () => b.exists("#declare-model-provider"));
await fill("#provider-id", "local");
await fill("#provider-name", "The box under the desk");
await fill("#provider-base-url", "http://127.0.0.1:8000/v1");
await choose("#provider-key-type", "generic_env_var");
await fill("#provider-models", "quality · Quality\nfast · Fast");
await b.click("#add-model-provider");
await b.waitFor("the provider listed", async () => b.exists('.provider-row[data-provider="local"][data-origin="declared"]'));

// --- The model this profile asks for --------------------------------------
step("the provider and the model are chosen for this profile");
await choose("#profile-model-provider", "local");
await saved("the provider");
await b.waitFor("the provider's models offered", async () =>
  (await b.evaluate(`[...(document.getElementById("profile-model")?.options ?? [])].map((o) => o.value).join(",")`)).includes("quality"));
await choose("#profile-model", "quality");
await saved("the model");
await b.waitFor("the key this provider takes is named", async () => b.exists("#provider-key"));

// --- The role, in the person's words --------------------------------------
step("the role is written");
await fill("#profile-role", "You are Ada.\nBe brief.");
await b.click("#save-role");
await saved("the role");

// --- A skill --------------------------------------------------------------
step("a skill is added");
await fill("#skill-name", "review");
await fill("#skill-description", "when asked to review");
await fill("#skill-body", "Read first.");
await b.click("#add-skill");
await b.waitFor("the skill listed", async () => b.exists('.skill-row[data-skill="review"]'));

// The rail's card says what this profile now asks for.
await b.waitFor("the profile card names the model", async () =>
  (await b.evaluate(`document.querySelector("#profile-card .k-chip")?.textContent ?? ""`)) === "quality");

// --- A session, and what the agent got ------------------------------------
step("a session starts with the model");
await b.click('[data-agent-tab="conversation"]');
await b.click("#open-new");
await b.waitFor("connection open", async () => (await b.textOf("connection-id")) !== "-", 300);
// The strip over the conversation shows the session's model - the echo
// fixture offers `fast` and `quality`, and the profile asked for `quality`,
// so the host chose it on the session and the strip reads it back.
await b.waitFor("the session runs on the chosen model", async () => (await valueOf("#session-model")) === "quality", 300);
await b.evaluate(`(() => {
  const field = document.getElementById("prompt-text");
  field.value = "what did you get";
})()`);
await b.click("#send");
await b.waitFor("the turn ends", async () => (await b.textOf("turn-outcome")).startsWith("turn:"), 300);
const reply = await b.evaluate(`[...document.querySelectorAll("#conversation .message.assistant .message-body")].map((n) => n.textContent).pop() ?? ""`);
let seen;
try {
  seen = JSON.parse(reply);
} catch {
  cleanup(1, `the echo's reply is not JSON: ${reply}`);
}
const modelVariable = seen?.environment?.model_environment?.ANTHROPIC_MODEL ?? null;
const sessionModel = seen?.configuration?.model ?? null;
if (modelVariable !== "quality") cleanup(1, `the model did not reach the agent's environment: ${JSON.stringify(seen?.environment)}`);
if (sessionModel !== "quality") cleanup(1, `the model was not set on the session: ${JSON.stringify(seen?.configuration)}`);
const route = await b.textOf("route");

// --- Come back to it --------------------------------------------------------
step("the page is reloaded and the session is found and resumed");
await b.send("Page.reload", {ignoreCache: true});
await b.waitFor("the agent space again", async () => b.exists('.app-shell[data-active-space="agent"]'));
await b.waitFor("the session listed", async () => b.exists(`#sessions .session-row[data-route-id=${JSON.stringify(route)}]`), 300);
await b.click(`#sessions .session-row[data-route-id=${JSON.stringify(route)}]`);
await b.waitFor("resumed", async () => (await b.textOf("connection-id")) !== "-" && (await b.textOf("route")) === route, 300);
await b.waitFor("what was said is back", async () =>
  (await b.evaluate(`document.querySelectorAll("#conversation .message").length`)) >= 2, 300);
await b.waitFor("the model is still the session's", async () => (await valueOf("#session-model")) === "quality", 300);

console.log(JSON.stringify({route, model_variable: modelVariable, session_model: sessionModel}));
await sleep(200);
await b.close();
process.exit(0);
