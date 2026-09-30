// Two people of a product that is not SWEM, each with agents of their own
// built in under a path of that product's server.

import {launchBrowser, sleep, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [ada, bo, adasAgents] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!ada || !bo || !adasAgents) {
  console.error("usage: built_in_driver.mjs <ada comes in at> <bo comes in at> <ada's agents>");
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
