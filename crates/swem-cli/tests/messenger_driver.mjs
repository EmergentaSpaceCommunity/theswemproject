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

const step = (name) => console.log(`step: ${name}`);
const [url, api] = process.argv.slice(2);
const browser = browserPath();
if (!url || !api) {
  console.error("usage: messenger_driver.mjs <url> <bot-api-address>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "messenger"});
/// What the walk gives the channel as the bot's token. Not a token of anything.
const TOKEN = "123456:walk-not-a-token-of-anything";
const OWNER = {id: 7, is_bot: false, first_name: "Ada", username: "ada"};
const STRANGER = {id: 9, is_bot: false, first_name: "Bob", username: "bob"};
let messageId = 0;

/// What the messenger's side does: somebody writes to the bot.
const written = async (from, text) => {
  messageId += 1;
  const response = await fetch(`${api}/_fixture/updates`, {
    method: "POST",
    headers: {"content-type": "application/json"},
    body: JSON.stringify({
      message: {
        message_id: messageId,
        from,
        chat: {id: from.id, type: "private", first_name: from.first_name},
        date: 0,
        text,
      },
    }),
  });
  if (!response.ok) cleanup(1, `the fixture refused an update: ${response.status}`);
};
/// What the bot sent, as the messenger recorded it: `{method, body}` in order.
const sent = async () => (await (await fetch(`${api}/_fixture/sent`)).json()).result ?? [];
const sentTo = (calls, chat, method = "sendMessage") =>
  calls.filter((call) => call.method === method && String(call.body?.chat_id) === String(chat));

const choose = async (selector, value) =>
  b.evaluate(`(() => {
    const field = document.querySelector(${JSON.stringify(selector)});
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(field, ${JSON.stringify(value)});
    field.dispatchEvent(new Event('change', {bubbles: true}));
  })()`);
const options = (selector) => b.evaluate(`[...(document.querySelector(${JSON.stringify(selector)})?.options ?? [])].map((o) => o.value)`);

step("the product opens");
const profile = await b.makeAgent("hands");
step("the agent is theirs");

// --- Providers → Channels: a bot, with the token the messenger gave -------
await b.goTo("#/providers/channels");
await b.waitFor("the channels page offers a bot", async () => b.exists("#channel-add-bot"), 100);
await b.click("#channel-add-bot");
await b.waitFor("the form for a bot", async () => b.exists("#channel-name"));
if (!(await options("#channel-package")).includes("telegram")) cleanup(1, `the product came with no Telegram channel: ${JSON.stringify(await options("#channel-package"))}`);
await b.fill("#channel-name", "My bot");
await choose("#channel-package", "telegram");
await b.fill("#channel-key", TOKEN);
if (!(await options("#channel-agent")).includes(profile)) cleanup(1, `the agent cannot be chosen to answer: ${JSON.stringify(await options("#channel-agent"))}`);
await choose("#channel-agent", profile);
await choose("#channel-guests", "nobody");
await b.click("#channel-more");
await b.waitFor("the address field", async () => b.exists("#channel-api-root"));
await b.fill("#channel-api-root", api);
await b.click("#channel-add");
await b.waitFor("the bot is running", async () => b.exists('.channel-row[data-running="true"]'), 150);
const row = () => b.evaluate(`document.querySelector('.channel-row')?.innerText ?? ""`);
if (!(await row()).includes("@swem_fixture_bot")) cleanup(1, `the bot was not looked at: ${await row()}`);
if ((await b.evaluate("document.body.innerText")).includes(TOKEN)) cleanup(1, "the page shows the token back");
const code = (await b.evaluate(`document.querySelector('.channel-code')?.textContent ?? ""`)).trim();
if (!/^\d{6}$/.test(code)) cleanup(1, `no code to say to the bot: ${JSON.stringify(code)}`);
step("the bot is added, looked at, and a code is shown");

// --- The person says the code to the bot, once ---------------------------
await written(OWNER, code);
await b.waitFor("the page shows the person paired", async () => b.exists('.channel-row[data-paired="true"]'), 150);
const welcome = sentTo(await sent(), OWNER.id);
if (!welcome.some((call) => String(call.body.text).includes("known here"))) cleanup(1, `the bot did not say the person is known: ${JSON.stringify(welcome)}`);
step("the person is known to the bot");

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
if (!String(line.body.text).includes("owner only")) cleanup(1, `the stranger got something else: ${JSON.stringify(line.body)}`);
await sleep(1500);
if (sentTo(await sent(), STRANGER.id).length !== 1) cleanup(1, "the stranger got more than one line");
if (sentTo(await sent(), STRANGER.id, "sendMessageDraft").length !== 0) cleanup(1, "the agent answered the stranger");
const chatsAfter = ((await b.ask("/api/chats")).body ?? []).length;
if (chatsAfter !== chatsBefore + 1) cleanup(1, `chats: ${chatsBefore} before, ${chatsAfter} after; the stranger should have opened none`);
step("a stranger got one line and opened nothing");

// --- The bot is removed from the page -------------------------------------
await b.goTo("#/providers/channels");
await b.waitFor("the bot listed", async () => b.exists(".channel-remove"), 100);
await b.click(".channel-remove");
await b.waitFor("the bot gone", async () => !(await b.exists(".channel-row")), 100);
step("the bot is removed");

console.log(JSON.stringify({code_length: code.length, chats: chatsAfter}));
console.log("messenger OK");
await sleep(200);
await b.close();
process.exit(0);
