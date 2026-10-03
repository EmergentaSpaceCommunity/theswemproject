// A person opens the Store, adds a catalog by address, installs an agent, an
// MCP server and a skill from it, makes the agent theirs, gives it the
// server and the skill, and the agent uses both.
//
// Every step is a real click on the running product; every consent is the
// product's own dialog, answered and kept for the walk to read.

import {launchBrowser, sleep, cleanup, browserPath} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, catalogUrl, agentId, registryId, nonce, laterCatalogUrl] = process.argv.slice(2);
const browser = browserPath();
if (!url || !catalogUrl || !agentId || !registryId || !nonce || !laterCatalogUrl) {
  console.error("usage: store_driver.mjs <url> <catalog-url> <agent-id> <registry-id> <nonce> <later-catalog-url>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "store"});

const setValue = (id, value, proto = "HTMLInputElement") => b.evaluate(`(() => {
  const field = document.getElementById(${JSON.stringify(id)});
  Object.getOwnPropertyDescriptor(window.${proto}.prototype, 'value').set.call(field, ${JSON.stringify(value)});
  field.dispatchEvent(new Event(${proto === "HTMLSelectElement" ? '"change"' : '"input"'}, {bubbles: true}));
})()`);
const text = (id) => b.evaluate(`(document.getElementById(${JSON.stringify(id)})||{textContent:""}).textContent`);

step("the product opens");
await b.goTo("#/store");
await b.waitFor("the registry's agents", async () =>
  b.exists(`.store-entry[data-kind="agent"][data-id="${registryId}"]`), 300);
step("the Store lists the registry's agents");
// And nothing that did not come with the product: this product has no
// Cycle beside it, so the Store lists no Cycle as "came with SWEM".
if (await b.exists('.store-entry[data-id="swem-cycle"]')) cleanup(1, "the Store lists a Cycle this product does not have");
// What did come: the tunnel beside the binary, and the tool it drives.
await b.waitFor("the tunnel that came with the product", async () => b.exists('.store-entry[data-id="cloudflare"]'), 100);
if (!(await b.exists('.store-entry[data-id="cloudflared"]'))) cleanup(1, "the Store does not offer the tool the tunnel drives");
step("no Cycle is listed as what came with SWEM; the tunnel and its tool are");

step("add a catalog by address");
await setValue("index-url", catalogUrl);
await b.click("#index-add");
await b.waitFor("the catalog's entries", async () =>
  b.exists('.store-entry[data-kind="server"][data-id="echo"]') && b.exists('.store-entry[data-kind="skill"][data-id="shout"]'), 300);
const indexes = await b.evaluate(`[...document.querySelectorAll("#store-indexes .index-row")].map(row => row.dataset.index)`);
step(`catalog added: ${JSON.stringify(indexes)}`);

// A kind nobody here takes is listed, said so, and cannot be installed.
const nobodys = await b.evaluate(`(() => {
  const row = document.querySelector('.store-entry[data-id="nothing"]');
  if (!row) return "no row";
  const button = row.querySelector(".store-install");
  return JSON.stringify({words: row.innerText, disabled: button?.disabled});
})()`);
const nobody = JSON.parse(nobodys === "no row" ? "{}" : nobodys);
if (!nobody.disabled || !/does not have/.test(nobody.words ?? "")) cleanup(1, `a kind nobody takes is not said so: ${nobodys}`);
step("a kind nobody here takes is said so and cannot be installed");

// One install at a time, each against its own consent question.
const install = async (kind, id, {also = null, update = false} = {}) => {
  await b.click(`.store-install[data-kind="${kind}"][data-id="${id}"]`);
  const question = await b.consent();
  if (!(update ? /^Update / : /^Install /).test(question) || !/described by/.test(question)) {
    cleanup(1, `the consent question for ${id} does not say what is fetched: ${JSON.stringify(question)}`);
  }
  if (also && !question.includes(`also installs what this requires: a skill ${also}`)) {
    cleanup(1, `the consent question for ${id} does not say what it also installs: ${JSON.stringify(question)}`);
  }
  await b.waitFor(`${id} installed`, async () =>
    (await b.exists(`.store-entry[data-kind="${kind}"][data-id="${id}"][data-installed="true"]`)) &&
    !/^(Reading|Installing) /.test(await text("store-status")), 600, 500);
  const status = await text("store-status");
  if (!/^Installed /.test(status)) cleanup(1, `installing ${id} ended with: ${status}`);
  step(`installed ${kind} ${id}: ${status}`);
};
await install("agent", registryId);
await install("server", "echo");
// The skill that requires another: one consent, both installed.
await install("skill", "polite", {also: "shout"});
if (!(await b.exists('.store-entry[data-kind="skill"][data-id="shout"][data-installed="true"]'))) {
  cleanup(1, "what polite requires was not installed with it");
}
step("what a skill requires was installed with it, in one consent");

// A later catalog with a newer version: an update is offered and taken.
await setValue("index-url", laterCatalogUrl);
await b.click("#index-add");
await b.waitFor("the update offered", async () =>
  b.evaluate(`/^Update to 1\\.1\\.0/.test(document.querySelector('.store-install[data-kind="skill"][data-id="shout"]')?.textContent ?? "")`), 300);
await install("skill", "shout", {update: true});
await b.waitFor("the newer version installed", async () =>
  b.evaluate(`/^Installed 1\\.1\\.0/.test(document.querySelector('.store-install[data-kind="skill"][data-id="shout"]')?.textContent ?? "")`), 300);
step("updated the skill to the newer version");

// Removed, after the person said so.
await b.click('.store-remove[data-kind="skill"][data-id="polite"]');
const removal = await b.consent();
if (!/^Remove /.test(removal)) cleanup(1, `removing asked: ${JSON.stringify(removal)}`);
await b.waitFor("polite removed", async () =>
  b.exists('.store-entry[data-kind="skill"][data-id="polite"][data-installed="false"]'), 300);
step("removed the skill that was not wanted");

// The agent is offered as this computer's, under the catalogue's own name.
step("make an agent of it");
await b.waitFor("the engine offered as this computer's", async () =>
  ((await b.ask("/api/onboarding")).body?.agents ?? []).some((one) => one.agent_id === agentId && one.available), 300);
const profile = await b.makeAgent(agentId);

step("give it the server and the skill");
await b.openAgent(profile, "settings");
await b.waitFor("the server declared from the Store", async () =>
  b.exists('#mcp-servers .server-row[data-server="echo"][data-origin="catalogue"]'), 300);
await b.click('.server-attach[data-server="echo"]');
await b.waitFor("the server attached", async () =>
  b.evaluate(`!!document.querySelector('.server-attach[data-server="echo"]')?.checked`), 200);
await b.waitFor("the installed skill offered", async () => b.exists('#skill-installed option[value="shout"]'), 300);
await setValue("skill-installed", "shout", "HTMLSelectElement");
await b.click("#skill-attach");
await b.waitFor("the skill on the profile", async () => b.exists('.skill-row[data-skill="shout"]'), 300);
step("attached: echo, and the skill shout");

step("a chat that uses both");
await b.openAgent(profile);
const say = async (message) => (await b.say(message)).at(-1);
const echoed = await say(JSON.stringify({tool: "echo", arguments: {nonce}, server: "echo"}));
if (!echoed.includes(nonce)) cleanup(1, `the server installed from the Store did not answer: ${JSON.stringify(echoed)}`);
step("the installed server answered the agent's call");
const skill = await say(JSON.stringify({read: ".claude/skills/shout/SKILL.md"}));
if (!skill.includes("Answer in capitals")) cleanup(1, `the skill is not where the agent reads it: ${JSON.stringify(skill)}`);
step("the installed skill is in the agent's skills folder");
await sleep(200);

console.log("store OK");
console.log(JSON.stringify({indexes, echoed, skill}));
cleanup(0, "store walk done");
