// The messenger beyond direct messages: the bot in a group and in a forum
// topic, a guest let into a chat from the page, the agent's question
// answered with a button, a file each way.
//
// Same fixture as the first messenger walk; the bot answers guests by
// invitation, so a group the owner opened is one the others may write in,
// and somebody alone with the bot waits until the owner lets them in.

import {launchBrowser, sleep, cleanup, browserPath} from "../../swem-host/tests/cdp_browser.mjs";
import {OWNER, STRANGER as GUEST, addBotAndPair, messenger} from "./messenger_common.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, api, secret] = process.argv.slice(2);
const browser = browserPath();
if (!url || !api || !secret) {
  console.error("usage: messenger_more_driver.mjs <url> <bot-api-address> <what-is-in-the-file>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "messenger-more"});
const tg = messenger(api);
const {written, pressed, sent, sentTo} = tg;
const GROUP = {id: -100, type: "supergroup", title: "Studio"};
const beside = 'aside[aria-label="About this chat"]';
const USAGE = "this fixture takes";
const answersIn = (calls, chat, saying = USAGE) => sentTo(calls, chat).filter((call) => String(call.body.text).includes(saying));

step("the product opens");
const profile = await b.makeAgent("hands");
await addBotAndPair(b, api, profile, {guests: "by_invitation", tg});
step("the bot is added and the person is known to it");

// --- A group the bot is in is a chat of several ---------------------------
await written(OWNER, "@swem_fixture_bot hello group", {chat: GROUP});
await b.openAgent(profile);
await b.waitFor("the group's chat on the agent's page", async () => b.pressText(".k-rail-item", "Studio"), 150, 200);
await b.waitFor("the agent answers in the group", async () => answersIn(await sent(), GROUP.id).length >= 1, 200, 200);
// Somebody else in the group writes to the bot, and is in the chat.
await written(GUEST, "@swem_fixture_bot hi from the group", {chat: GROUP});
await b.waitFor("the guest is among the members", async () =>
  ((await b.chat())?.chat.members ?? []).some((member) => member.kind === "guest" && member.name === GUEST.first_name), 150);
await b.waitFor("the list beside says so", async () => (await b.evaluate(`document.querySelector('${beside}')?.innerText ?? ""`)).includes(GUEST.first_name), 100);
await b.waitFor("the agent answers them in the group", async () => answersIn(await sent(), GROUP.id).length >= 2, 200, 200);
step("the group is a chat of several, and whoever writes in it is in it");

// --- A forum topic is a chat of its own ------------------------------------
await written(OWNER, "@swem_fixture_bot in the topic", {chat: GROUP, extra: {is_topic_message: true, message_thread_id: 55}});
await b.waitFor("the topic's chat on the agent's page", async () => b.pressText(".k-rail-item", "Studio · topic 55"), 150, 200);
await b.waitFor("the agent answers in the topic", async () =>
  answersIn(await sent(), GROUP.id).some((call) => call.body.message_thread_id === 55), 200, 200);
step("a topic is a chat of its own, answered in the topic");

// --- A guest alone with the bot waits until let in -------------------------
await written(OWNER, "hello alone");
await b.waitFor("the bot's own chat", async () => b.pressText(".k-rail-item", "Fixture bot"), 150, 200);
await b.waitFor("the agent answers alone", async () => answersIn(await sent(), OWNER.id).length >= 1, 200, 200);
await written(GUEST, "can I talk to it too?");
await b.waitFor("the guest is told to wait", async () =>
  sentTo(await sent(), GUEST.id).some((call) => String(call.body.text).includes("let you into a chat")), 150);
// The bot's row on the Channels page says who waits; one press lets them
// into the chat the person has with the bot.
await b.goTo("#/providers/channels");
await b.waitFor("the guest is shown waiting at the bot", async () => b.exists(".channel-row [data-waiting]"), 100);
if (!(await b.pressText(".channel-row [data-waiting] button", "Let in"))) cleanup(1, "the guest cannot be let in");
await b.waitFor("nobody waits any more", async () => !(await b.exists(".channel-row [data-waiting]")), 100);
await b.openAgent(profile);
await b.waitFor("the bot's own chat again", async () => b.pressText(".k-rail-item", "Fixture bot"), 150, 200);
await b.waitFor("the guest is in the chat", async () =>
  ((await b.chat())?.chat.members ?? []).some((member) => member.kind === "guest"), 100);
await b.waitFor("who is in the chat is shown beside it", async () => (await b.evaluate(`document.querySelector('${beside}')?.innerText ?? ""`)).includes(GUEST.first_name), 100);
await written(GUEST, "hello from the guest");
await b.waitFor("the owner sees the guest's words on their side", async () =>
  sentTo(await sent(), OWNER.id).some((call) => String(call.body.text).includes("<b>Bob:</b>") && String(call.body.text).includes("hello from the guest")), 150);
await b.waitFor("the agent's answer reaches the guest", async () => answersIn(await sent(), GUEST.id).length >= 1, 200, 200);
await b.waitFor("the guest's words on the page as theirs", async () =>
  ((await b.chat())?.messages ?? []).some((message) => message.text === "hello from the guest"), 100);
step("a guest was let in from the page, and is reached where they wrote from");

// --- The agent's question, answered with a button ---------------------------
await written(OWNER, JSON.stringify({ask: "the thing it wants to do"}));
await b.waitFor("the question reaches the messenger as buttons", async () =>
  sentTo(await sent(), OWNER.id).some((call) => JSON.stringify(call.body.reply_markup ?? {}).includes("Allow once")), 200, 200);
const asked = sentTo(await sent(), OWNER.id).find((call) => JSON.stringify(call.body.reply_markup ?? {}).includes("Allow once"));
if (!String(asked.body.text).includes("the thing it wants to do")) cleanup(1, `the question does not say what is asked: ${JSON.stringify(asked.body)}`);
const button = asked.body.reply_markup.inline_keyboard.flat().find((one) => one.text === "Allow once");
await pressed(OWNER, {id: OWNER.id, type: "private"}, button.callback_data);
await b.waitFor("the agent went on after the answer", async () =>
  answersIn(await sent(), OWNER.id, "names no tool").length >= 1, 200, 200);
step("the agent asked, the person answered with a button, the agent went on");

// --- A file in, a file out --------------------------------------------------
await written(OWNER, undefined, {extra: {
  document: {file_id: "doc1", file_name: "notes.txt", file_size: secret.length, mime_type: "text/plain"},
  caption: JSON.stringify({read: "inbox/notes.txt"}),
}});
await b.waitFor("the agent read the file that came with the message", async () =>
  answersIn(await sent(), OWNER.id, secret).length >= 1, 200, 200);
await b.waitFor("the file is on the message on the page", async () =>
  (await b.evaluate(`[...document.querySelectorAll('.k-chip')].map((one) => one.innerText).join('|')`)).includes("notes.txt"), 100);
await written(OWNER, JSON.stringify({write: "outbox/reply.txt", text: "what the agent hands back"}));
await b.waitFor("what the agent put out reaches the messenger", async () =>
  sentTo(await sent(), OWNER.id, "sendDocument").length >= 1, 200, 200);
step("a file came in with a message and one went out with the answer");

console.log(JSON.stringify({sent: (await sent()).length}));
console.log("messenger more OK");
await sleep(200);
await b.close();
process.exit(0);
