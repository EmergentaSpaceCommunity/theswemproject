// A Workbench served at an address has a door the messenger can deliver
// to, instead of being asked: the person switches the bot to it on the
// Channels page, the messenger is told where (and a secret only the two of
// them know), a delivery at the door reaches the chat, one without the
// secret is refused, and the bot can be switched back.
//
// The Workbench is served at `localhost`, so the person signs in with a
// passkey first, as on a server of their own.

import {launchBrowser, sleep, cleanup, browserPath} from "../../swem-host/tests/cdp_browser.mjs";
import {OWNER, addBotAndPair, messenger} from "./messenger_common.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, word, api] = process.argv.slice(2);
const browser = browserPath();
if (!url || !word || !api) {
  console.error("usage: messenger_door_driver.mjs <address> <word> <bot-api-address>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "messenger-door", door: true});
const tg = messenger(api);
const {sent, sentTo} = tg;
const USAGE = "this fixture takes";

const says = (what, selector, text) =>
  b.waitFor(what, async () => b.evaluate(
    `[...document.querySelectorAll(${JSON.stringify(selector)})].some((one) => one.innerText.includes(${JSON.stringify(text)}))`), 200);
const write = async (label, text) => {
  const found = await b.evaluate(`(() => {
    const field = [...document.querySelectorAll("label")]
      .find((one) => one.innerText.includes(${JSON.stringify(label)}))?.querySelector("input");
    if (!field) return false;
    field.focus();
    field.select();
    return true;
  })()`);
  if (!found) cleanup(1, `no field is called "${label}"`);
  await b.send("Input.insertText", {text});
  await sleep(150);
};

// --- Sign in, as on a server of one's own ---------------------------------
step("the first start asks for the word");
await b.holdPasskeys();
await says("the choice at the door", "h1", "Make it yours");
if (!(await b.pressText("button", "This device is mine"))) cleanup(1, "the door offers no way to make it mine");
await says("the first start", "h1", "Make this Workbench yours");
await write("The word it printed", word.toUpperCase().replaceAll("-", " "));
await write("A name for this device", "Laptop");
if (!(await b.pressText("button", "Register this device"))) cleanup(1, "the device cannot be registered");
await says("the codes", "h1", "Codes to come back with");
await b.evaluate(`document.querySelector('input[type="checkbox"]').click()`);
if (!(await b.pressText("button", "Open the Workbench"))) cleanup(1, "the Workbench does not open");
await says("the Workbench", 'nav[aria-label="Workbench"]', "Settings");
step("signed in with a passkey");

// --- The bot, asked at first --------------------------------------------
const profile = await b.makeAgent("hands");
await addBotAndPair(b, api, profile, {guests: "nobody", tg});
await b.goTo("#/providers/channels");
await b.waitFor("the bot asks the messenger", async () => b.exists('.channel-row [data-reach="pull"]'), 100);
step("the bot is added, and asks the messenger for what is new");

// --- Switched to the door --------------------------------------------------
if (!(await b.pressText('.channel-row [data-reach="pull"] button', "Have it delivered"))) cleanup(1, "the door is not offered on a served Workbench");
await b.waitFor("the bot is reached at the door", async () => b.exists('.channel-row [data-reach="door"]'), 150);
await b.waitFor("the messenger was told where", async () => (await sent()).some((call) => call.method === "setWebhook"), 100);
const told = (await sent()).filter((call) => call.method === "setWebhook").at(-1).body;
if (!String(told.url).includes("/api/channels/") || !String(told.url).endsWith("/receive")) cleanup(1, `the messenger was told a strange door: ${JSON.stringify(told)}`);
if (!told.secret_token || String(told.secret_token).length < 16) cleanup(1, `the door has no secret: ${JSON.stringify(told)}`);
const caption = await b.evaluate(`document.querySelector('.channel-row [data-reach="door"]')?.innerText ?? ""`);
if (!caption.includes("Delivered to a door")) cleanup(1, `the row does not say so: ${caption}`);
if (caption.includes(told.secret_token)) cleanup(1, "the page shows the door's secret");
step("the messenger was told the door and a secret; the page says so");

// --- A delivery at the door reaches the chat; one without the secret does not
const before = (await sent()).length;
const deliver = async (secret, text) => fetch(told.url, {
  method: "POST",
  headers: {"content-type": "application/json", ...(secret ? {"x-telegram-bot-api-secret-token": secret} : {})},
  body: JSON.stringify({update_id: 900 + before, message: {message_id: 900, from: OWNER, chat: {id: OWNER.id, type: "private"}, date: 0, text}}),
});
const refused = await deliver("not-the-secret", "a stranger at the door");
if (refused.ok) cleanup(1, `a delivery without the secret was taken in: ${refused.status}`);
const taken = await deliver(told.secret_token, "hello by the door");
if (!taken.ok) cleanup(1, `the delivery was refused: ${taken.status} ${await taken.text()}`);
await b.waitFor("the agent answers what came by the door", async () =>
  sentTo((await sent()).slice(before), OWNER.id).some((call) => String(call.body.text).includes(USAGE)), 200, 200);
await b.openAgent(profile);
await b.waitFor("the words on the page", async () => (await b.said()).some((text) => text === "hello by the door"), 100);
if ((await b.said()).some((text) => text.includes("a stranger at the door"))) cleanup(1, "the refused delivery reached the chat");
if ((await sent()).slice(before).some((call) => call.method === "getUpdates")) cleanup(1, "the bot still asks the messenger while reached at the door");
step("a delivery at the door reached the chat, one without the secret did not, and nothing was asked for");

// --- And back to asking -----------------------------------------------------
await b.goTo("#/providers/channels");
if (!(await b.pressText('.channel-row [data-reach="door"] button', "Ask instead"))) cleanup(1, "the bot cannot be switched back");
await b.waitFor("the bot asks again", async () => b.exists('.channel-row [data-reach="pull"]'), 150);
await b.waitFor("the messenger was told to deliver no more", async () =>
  (await sent()).slice(before).some((call) => call.method === "deleteWebhook"), 100);
step("switched back to asking");

console.log("messenger door OK");
await sleep(200);
await b.close();
process.exit(0);
