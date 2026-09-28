// How a call that brought an App can end, in the chat it was made in.
//
// Three chats with one agent. In the first the tool answers with an error,
// in the second the agent calls the tool off, and the App is given each
// ending and acknowledges it. In the third the person closes the App while
// the tool still works: the call is the agent's, and closing what shows it
// ends nothing.

import {writeFileSync} from "node:fs";

import {launchBrowser, cleanup} from "./cdp_browser.mjs";

const [browser, url, delayRelease] = process.argv.slice(2);
if (!browser || !url || !delayRelease) {
  console.error("usage: node workbench_shell_apps_terminal_cdp_driver.mjs <browser> <url> <delay-release>");
  process.exit(2);
}
const b = await launchBrowser({browser, url, label: "terminal"});

const panel = '.w-apps:not([hidden])';
const status = () => b.evaluate(`[...document.querySelectorAll('${panel} > .k-caption')].map((one) => one.textContent).join(" ")`);
const happened = async () => (await b.chat())?.events ?? [];
const ended = async (how) => (await happened()).some((event) =>
  event.kind === "chat/delivery" && event.payload?.state === how);

const agent = await b.agentOf("apps-main");
if (!agent) cleanup(1, "there is no agent apps-main");

/// Begin another chat with the agent by asking it for something.
const ask = async (prompt) => {
  await b.goTo(`#/agents/${agent}/chats/new`);
  await b.say(JSON.stringify(prompt), {answered: false});
  await b.waitFor("the chat begun", async () => {
    const shown = (await b.evaluate("location.hash")).split("/chats/")[1] ?? "new";
    return shown !== "new";
  }, 100);
  return (await b.chat()).chat.chat_id;
};
const closeAndSleep = async (chat) => {
  if (await b.exists(`${panel} #app-frame`)) {
    if (!(await b.pressText(`${panel} button`, "Close it"))) cleanup(2, "the App cannot be closed");
    await b.waitFor("the App closed", async () =>
      (await happened()).some((event) => event.kind === "host/app_closed"));
  }
  await b.putToSleep(chat, agent);
};

const error_chat = await ask({
  fixture: "mcp-apps-error-v0.1", server: "notes",
  nonce: "browser-tool-error-0.57", text: "deliver exact isError result",
});
await b.waitFor("the tool's error given to the App", async () =>
  (await status()).includes("agent App tool error delivered"), 600);
await b.waitFor("the App's acknowledgement of the error", async () =>
  (await happened()).some((event) => event.kind === "host/app_tool_call"));
await b.waitFor("the turn ended by itself", async () => ended("done"), 300);
await closeAndSleep(error_chat);

const cancel_chat = await ask({
  fixture: "mcp-apps-cancel-v0.1", server: "notes",
  nonce: "browser-cancel-0.57", text: "native MCP cancellation",
});
await b.waitFor("the calling off given to the App", async () =>
  (await status()).includes("agent App cancellation delivered"), 600);
await b.waitFor("the App's acknowledgement of the calling off", async () =>
  (await happened()).some((event) => event.kind === "host/app_tool_call"));
await b.waitFor("the turn called off", async () => ended("stopped"), 300);
await closeAndSleep(cancel_chat);

const delay_chat = await ask({
  fixture: "mcp-apps-delay-v0.1", server: "notes",
  nonce: "browser-teardown-race-0.57", text: "View teardown cannot own this call",
  delay_ms: 3000,
});
await b.waitFor("the App of a call still going", async () =>
  (await b.exists(`${panel} #app-frame`)) && (await status()).includes("app ready"), 600);
if (!(await b.pressText(`${panel} button`, "Close it"))) cleanup(2, "the App of a call still going cannot be closed");
await b.waitFor("the App closed", async () =>
  (await happened()).some((event) => event.kind === "host/app_closed"));
writeFileSync(delayRelease, "view-closed\n", {flag: "wx"});
await b.waitFor("the call ended by itself after its App was closed", async () => ended("done"), 300);
if (await b.exists(`${panel} #app-frame`)) cleanup(1, "the App a person closed came back by itself");
await closeAndSleep(delay_chat);

console.log(JSON.stringify({error_chat, error_agent: agent, cancel_chat, cancel_agent: agent, delay_chat, delay_agent: agent}));
cleanup(0);
