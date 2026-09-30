// Two people of a product that is not SWEM, each with agents of their own
// built in under a path of that product's server.

import {launchBrowser, sleep, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [ada, bo, adasAgents, catalogUrl] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!ada || !bo || !adasAgents || !catalogUrl) {
  console.error("usage: built_in_driver.mjs <ada comes in at> <bo comes in at> <ada's agents> <catalog-url>");
  process.exit(2);
}

const b = await launchBrowser({browser, url: ada, label: "built-in"});
const text = (selector) => b.evaluate(`document.querySelector(${JSON.stringify(selector)})?.innerText.trim() ?? ""`);

step("Ada is let in by the product and finds her Workbench under its path");
const where = await b.evaluate("location.pathname");
if (where !== "/people/ada/agents/") throw new Error(`Ada's Workbench is at ${where}`);
if ((await text(".w-brand")) !== "Example") throw new Error(`the page is called ${await text(".w-brand")}`);
if ((await text(".w-you .k-name")) !== "Ada") throw new Error(`the owner is ${await text(".w-you .k-name")}`);
if ((await b.evaluate("document.title")) !== "Example") throw new Error(`the title is ${await b.evaluate("document.title")}`);

step("she makes an agent and talks to it");
await b.makeAgent("hands");
const said = await b.say("hello from a product of my own");
if (!said.some((one) => one.includes("hello from a product of my own"))) throw new Error(`nothing was said back: ${JSON.stringify(said)}`);

step("her agent was handed the product's own server, for her");
const handed = (await b.ask(`/api/profiles/hands/servers`)).body;
if (!handed?.given?.includes("example")) throw new Error(`the product's server was not given: ${JSON.stringify(handed)}`);
await b.openAgent("hands", "settings");
await b.waitFor("the server given by the product", async () => b.exists('[data-server="example"][data-given="true"]'), 100);

step("the product's Store takes a kind of the product's own: a helper");
const setValue = (id, value) => b.evaluate(`(() => {
  const field = document.getElementById(${JSON.stringify(id)});
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(field, ${JSON.stringify(value)});
  field.dispatchEvent(new Event("input", {bubbles: true}));
})()`);
const status = () => b.evaluate(`(document.getElementById("store-status")||{textContent:""}).textContent`);
await b.goTo("#/store");
await b.waitFor("the Store", async () => b.exists("#index-url"), 100);
await setValue("index-url", catalogUrl);
await b.click("#index-add");
await b.waitFor("the helpers listed", async () => b.exists('.store-entry[data-kind="example/helper@1"][data-id="echoes"]'), 300);
const kinds = JSON.parse(await b.evaluate(`JSON.stringify([...document.querySelectorAll("#store-kind option")].map((one) => one.textContent))`));
if (!kinds.includes("Helpers")) throw new Error(`the product's kind is not listed in its own words: ${JSON.stringify(kinds)}`);
const helperWords = await b.evaluate(`document.querySelector('.store-entry[data-id="echoes"] small')?.textContent ?? ""`);
if (!helperWords.startsWith("a helper for Example")) throw new Error(`a helper is not called by the product's words: ${helperWords}`);
const cameWith = await b.evaluate(`[...document.querySelectorAll(".store-install")].map((one) => one.textContent).find((words) => words.startsWith("Came with")) ?? ""`);
if (cameWith && cameWith !== "Came with Example") throw new Error(`what came with the product is said to have come with another: ${cameWith}`);

step("a helper that does not answer the product's shape is refused in the product's words");
await b.click('.store-install[data-kind="example/helper@1"][data-id="takes"]');
await b.consent();
await b.waitFor("the refusal", async () => /is not a helper for Example/.test(await status()), 600, 500);
const refused = await status();
if (!refused.includes("does not answer `echo`")) throw new Error(`the refusal does not say what is short: ${refused}`);
if (await b.exists('.store-entry[data-id="takes"][data-installed="true"]')) throw new Error("a refused helper was installed");

step("a helper that answers the shape is taken, and her agent is given it");
await b.click('.store-install[data-kind="example/helper@1"][data-id="echoes"]');
const question = await b.consent();
if (!/^Install a helper for Example echoes 0\.1\.0\?/.test(question)) throw new Error(`the consent question: ${question}`);
await b.waitFor("echoes installed", async () =>
  (await b.exists('.store-entry[data-id="echoes"][data-installed="true"]')) && !/^(Reading|Installing) /.test(await status()), 600, 500);
const taken = await status();
if (!taken.startsWith("Installed The helper that echoes. Every agent of yours has it")) throw new Error(`installing ended with: ${taken}`);
const givenNow = (await b.ask(`/api/profiles/hands/servers`)).body;
if (!givenNow?.given?.includes("echoes")) throw new Error(`the helper was not given to her agent: ${JSON.stringify(givenNow)}`);

step("and removed from the Store, it is given no more");
await b.click('.store-remove[data-kind="example/helper@1"][data-id="echoes"]');
const asked = await b.consent();
if (!/^Remove a helper for Example echoes/.test(asked)) throw new Error(`the removal question: ${asked}`);
await b.waitFor("echoes removed", async () => b.exists('.store-entry[data-id="echoes"][data-installed="false"]'), 300);
const givenAfter = (await b.ask(`/api/profiles/hands/servers`)).body;
if (givenAfter?.given?.includes("echoes")) throw new Error(`the removed helper is still given: ${JSON.stringify(givenAfter)}`);

step("Bo is let in and finds none of Ada's");
await b.evaluate(`location.href = ${JSON.stringify(bo)}`);
await b.waitFor("Bo's Workbench", async () => b.evaluate(`location.pathname === "/people/bo/agents/" && document.querySelector(".w-you .k-name")?.innerText.trim() === "Bo"`), 200);
await sleep(800);
const agents = await b.evaluate(`JSON.stringify([...document.querySelectorAll('nav[aria-label="Workbench"] .k-rail-item .k-name')].map((one) => one.innerText.trim()))`);
if (JSON.parse(agents).includes("hands")) throw new Error(`Bo sees Ada's agent: ${agents}`);
const bosChats = (await b.ask("/api/chats")).body ?? [];
if (bosChats.length !== 0) throw new Error(`Bo sees chats: ${JSON.stringify(bosChats)}`);

step("Bo at Ada's address is refused by the product itself");
await b.evaluate(`location.href = ${JSON.stringify(adasAgents)}`);
await b.waitFor("the product's own refusal", async () => b.evaluate(`document.body.innerText.includes("These are Ada's agents")`), 100);

console.log("built in OK");
cleanup(0, "built in OK");
