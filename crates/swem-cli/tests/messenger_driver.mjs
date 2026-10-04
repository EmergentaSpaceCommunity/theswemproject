// A person reaches their agent from a messenger.
//
// They add a bot on Providers → Channels with the token the messenger gave
// them, are shown a code, say it to the bot once, and from then on what they
// write to the bot is said in a chat with their agent, and what the agent
// answers is drafted while it is written and sent when done. A stranger who
// writes to the bot gets one line and no chat.
//
// The messenger is a Bot API that is a fixture, started by the test and
// reachable at `api`: the walk types its address as the channel's "Bot API
// address" (the setting a local Bot API server needs), and talks to the
// fixture's side door the way a phone would talk to Telegram.

import {launchBrowser, sleep, cleanup, browserPath} from "../../swem-host/tests/cdp_browser.mjs";
import {OWNER, STRANGER, addBotAndPair, messenger} from "./messenger_common.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, api] = process.argv.slice(2);
const browser = browserPath();
if (!url || !api) {
  console.error("usage: messenger_driver.mjs <url> <bot-api-address>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "messenger"});
const tg = messenger(api);
const {written, sent, sentTo} = tg;

step("the product opens");
const profile = await b.makeAgent("hands");
step("the agent is theirs");

// --- Providers → Channels: a bot, with the token the messenger gave; the
// person says the code to the bot, once ----------------------------------
const code = await addBotAndPair(b, api, profile, {guests: "nobody", tg});
step("the bot is added, looked at, the code shown and said: the person is known to the bot");

// --- A message to the bot is a chat with the agent -----------------------
const chatsBefore = ((await b.ask("/api/chats")).body ?? []).length;
await written(OWNER, "hello from my phone");
await b.openAgent(profile);
await b.waitFor("the chat with the bot on the agent's page", async () => b.pressText(".k-rail-item", "Fixture bot"), 150, 200);
await b.waitFor("the message, marked as from the messenger", async () => b.exists('[data-through^="channel:"]'), 150);
const shown = await b.evaluate(`document.querySelector('[data-through^="channel:"]')?.innerText ?? ""`);
if (!shown.includes("hello from my phone") || !shown.includes("from a messenger")) cleanup(1, `the message is not shown as from the messenger: ${JSON.stringify(shown)}`);
step("what was written to the bot is in the chat, marked as from the messenger");

// The agent answers: drafted while it is written, sent when done, as HTML.
const finalAnswer = (calls) => sentTo(calls, OWNER.id).find((call) => String(call.body.text).includes("this fixture takes"));
await b.waitFor("the agent's answer reaches the messenger", async () => {
  const calls = await sent();
  return sentTo(calls, OWNER.id, "sendMessageDraft").length > 0 && finalAnswer(calls) !== undefined;
}, 200, 200);
const answered = finalAnswer(await sent());
if (answered.body.parse_mode !== "HTML") cleanup(1, `the answer is not rich: ${JSON.stringify(answered.body)}`);
await b.waitFor("the same answer on the page", async () => (await b.said()).some((text) => text.includes("this fixture takes")), 100);
step("the agent answered on the messenger and on the page alike");

// --- A stranger gets one line, and no chat --------------------------------
await written(STRANGER, "hello, can you help me too?");
await b.waitFor("the stranger gets a line", async () => sentTo(await sent(), STRANGER.id).length > 0, 150);
const line = sentTo(await sent(), STRANGER.id)[0];
if (!String(line.body.text).includes("allow you to speak")) cleanup(1, `the stranger got something else: ${JSON.stringify(line.body)}`);
await sleep(1500);
if (sentTo(await sent(), STRANGER.id).length !== 1) cleanup(1, "the stranger got more than one line");
if (sentTo(await sent(), STRANGER.id, "sendMessageDraft").length !== 0) cleanup(1, "the agent answered the stranger");
const chatsAfter = ((await b.ask("/api/chats")).body ?? []).length;
if (chatsAfter !== chatsBefore + 1) cleanup(1, `chats: ${chatsBefore} before, ${chatsAfter} after; the stranger should have opened none`);
step("a stranger got one line and opened nothing");

// --- Not served at an address: no door, and the page says why ------------
await b.goTo("#/providers/channels");
await b.waitFor("the bot listed", async () => b.exists(".channel-remove"), 100);
const reach = await b.evaluate(`document.querySelector('.channel-row [data-reach="pull"]')?.innerText ?? ""`);
if (!reach.includes("served at an address")) cleanup(1, `the row does not say why the messenger cannot deliver here: ${JSON.stringify(reach)}`);
if (await b.evaluate(`[...document.querySelectorAll('.channel-row [data-reach] button')].some((one) => one.innerText.includes("Have it delivered"))`)) cleanup(1, "a door is offered on a Workbench not served at an address");
step("not served at an address, the bot asks the messenger, and the page says why there is no door");

// --- The bot is removed from the page -------------------------------------
await b.click(".channel-remove");
await b.waitFor("the bot gone", async () => !(await b.exists(".channel-row")), 100);
step("the bot is removed");

console.log(JSON.stringify({code_length: code.length, chats: chatsAfter}));
console.log("messenger OK");
await sleep(200);
await b.close();
process.exit(0);
