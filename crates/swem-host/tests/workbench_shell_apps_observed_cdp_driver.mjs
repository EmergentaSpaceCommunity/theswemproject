// The App a tool of the agent brings, in its chat.
//
// A person asks the agent for something; a tool it calls brings an App, and
// the panel beside the chat shows it with what the tool was given and what
// it answered. The person acknowledges it with a real click inside the
// sandboxed frame; the receipts the server wrote say what the App knew.

import {launchBrowser, cleanup, sleep} from "./cdp_browser.mjs";

const [browser, url] = process.argv.slice(2);
if (!browser || !url) {
  console.error("usage: node workbench_shell_apps_observed_cdp_driver.mjs <browser> <url>");
  process.exit(2);
}
const b = await launchBrowser({browser, url, label: "observed"});

const panel = '.w-apps:not([hidden])';
const status = () => b.evaluate(`[...document.querySelectorAll('${panel} > .k-caption')].map((one) => one.textContent).join(" ")`);
const clickInApp = async (x, y) => {
  const rect = JSON.parse(await b.evaluate(`JSON.stringify(document.getElementById("app-frame").getBoundingClientRect())`));
  await b.clickAt(rect.left + x, rect.top + y);
};
const happened = async () => (await b.chat())?.events ?? [];

const agent = await b.openAgent("apps-main");
await b.say(JSON.stringify({
  fixture: "mcp-apps-note-v0.1",
  server: "notes",
  nonce: "browser-agent-nonce-0.55",
  text: "exact native call projected through AppBridge",
}), {answered: false});
// A whole turn (the agent, the observed server, the host's projection, the
// delivery into the frame) takes seconds on a quiet machine and far longer
// on a loaded one.
await b.waitFor("what the tool brought, shown without being asked for", async () =>
  (await b.exists(`${panel} #app-frame`)) && (await status()).includes("agent App result delivered"), 600);
const opened = (await happened()).filter((event) => event.kind === "host/app_opened").pop()?.payload;
if (opened?.uri !== "ui://apps-fixture/notes") cleanup(2, `the call opened another App: ${JSON.stringify(opened)}`);

await sleep(500);
// The App enables its acknowledgement when the result lands in its frame,
// a beat after the host says so; a click that came too early is made again.
const acknowledged = async () => (await happened()).some((event) => event.kind === "host/app_tool_call");
for (let attempt = 0; attempt < 8 && !(await acknowledged()); attempt += 1) {
  await clickInApp(130, 228);
  for (let tick = 0; tick < 10 && !(await acknowledged()); tick += 1) await sleep(200);
}
await b.waitFor("the acknowledgement", acknowledged);
await b.waitFor("the turn over", async () => ((await b.chat())?.deliveries ?? []).length === 0, 300);
if (!(await b.pressText(`${panel} button`, "Close it"))) cleanup(2, "the App cannot be closed");
const chat = (await b.chat()).chat.chat_id;
await b.putToSleep(chat, agent);

console.log(JSON.stringify({main_chat: chat, main_agent: agent, app_server: opened.server}));
cleanup(0);
