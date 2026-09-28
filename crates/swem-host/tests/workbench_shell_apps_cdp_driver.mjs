// The Apps an agent's servers bring, in its chat.
//
// A person opens the panel beside the chat, opens an App, and clicks inside
// it: the click is a real one, inside the sandboxed frame, and what it did is
// read from the receipt the server wrote. The agent is put to sleep and the
// App opened again is the same App. A hostile App is refused everything it
// tries, and the chat goes on afterwards.

import {launchBrowser, cleanup, sleep} from "./cdp_browser.mjs";

const [browser, url] = process.argv.slice(2);
if (!browser || !url) {
  console.error("usage: node workbench_shell_apps_cdp_driver.mjs <browser> <url>");
  process.exit(2);
}
const b = await launchBrowser({browser, url, label: "apps"});

const panel = '.w-apps:not([hidden])';
const status = () => b.evaluate(`[...document.querySelectorAll('${panel} > .k-caption')].map((one) => one.textContent).join(" ")`);
const happened = async (chat) => ((await b.ask(`/api/chats/${chat}?limit=2000`)).body?.events ?? []);
const kinds = async (chat) => (await happened(chat)).map((event) => event.kind);
const clickInApp = async (x, y) => {
  const rect = JSON.parse(await b.evaluate(`JSON.stringify(document.getElementById("app-frame").getBoundingClientRect())`));
  await b.clickAt(rect.left + x, rect.top + y);
};

/// Go to an agent, start its chat with a word, and open its Apps.
const withApps = async (profile, word) => {
  const agent = await b.openAgent(profile);
  await b.say(word);
  const chat = (await b.chat()).chat.chat_id;
  if (!(await b.pressText(".w-composer button", "Apps"))) cleanup(2, "the composer has no Apps");
  await b.waitFor("the Apps its servers bring", async () => b.exists(`${panel} .k-rail-item`), 300);
  return {agent, chat};
};
const openApp = async () => {
  await b.waitFor("the Apps its servers bring", async () => b.exists(`${panel} .k-rail-item`), 300);
  const listed = await b.evaluate(`document.querySelector('${panel} .k-rail-item')?.title ?? ""`);
  await b.click(`${panel} .k-rail-item`);
  await b.waitFor("the App ready", async () =>
    (await b.exists("#app-frame")) && (await status()).includes("app ready"), 300);
  return listed;
};
const closeApp = async (chat) => {
  const before = (await kinds(chat)).filter((kind) => kind === "host/app_closed").length;
  if (!(await b.pressText(`${panel} button`, "Close it"))) cleanup(2, "the open App cannot be closed");
  await b.waitFor("the App closed", async () =>
    (await kinds(chat)).filter((kind) => kind === "host/app_closed").length > before);
};
const asleep = (chat, agent) => b.putToSleep(chat, agent);

const main = await withApps("apps-main", "hello");
const listedUri = await openApp();
const opened = (await happened(main.chat)).filter((event) => event.kind === "host/app_opened").pop()?.payload;
if (opened?.uri !== listedUri) cleanup(2, `opened ${opened?.uri}, listed ${listedUri}`);

await sleep(500);
await clickInApp(170, 75);
await b.send("Input.insertText", {text: "browser-app-nonce"});
await clickInApp(170, 125);
await b.send("Input.insertText", {text: "clicked inside a real sandboxed App"});
await clickInApp(100, 178);
await b.waitFor("what the click did, in the record", async () => (await kinds(main.chat)).includes("host/app_tool_call"));

// Asleep and awake again: the App opened again is the same App.
await closeApp(main.chat);
await asleep(main.chat, main.agent);
await openApp();
const again = (await happened(main.chat)).filter((event) => event.kind === "host/app_opened").pop()?.payload;
if (again?.uri !== opened.uri || again?.server !== opened.server) {
  cleanup(2, `the App is another one after the agent slept: ${JSON.stringify(again)}`);
}
await closeApp(main.chat);
await asleep(main.chat, main.agent);

// The hostile App: refused whatever it tries, and the chat goes on.
const hostile = await withApps("apps-hostile", "hello");
await b.click(`${panel} .k-rail-item`);
await b.waitFor("its attempts refused", async () =>
  (await happened(hostile.chat)).filter((event) => event.kind === "host/app_tool_call" && event.payload?.decision === "refused").length >= 3, 300);
const after = await b.say("still alive after the hostile app");
if (after.length < 4) cleanup(1, `the chat did not go on after the hostile App: ${JSON.stringify(after)}`);
await asleep(hostile.chat, hostile.agent);

console.log(JSON.stringify({
  main_chat: main.chat,
  main_agent: main.agent,
  hostile_chat: hostile.chat,
  hostile_agent: hostile.agent,
  app_server: opened.server,
  app_uri: opened.uri,
  app_permissions: JSON.stringify(opened.permissions ?? null),
}));
cleanup(0);
