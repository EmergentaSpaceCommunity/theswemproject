// The Workbench's own page inside the messenger, from a Workbench served
// at an address: the bot's button carries that address; opened from it,
// the page comes in by the messenger's signature - not the passkey - and is
// drawn for somebody from a messenger: the agent's files, the chat, a file
// of any size to the agent. A signature that is not the messenger's lets
// nobody in; opened by nobody at all, the page is the door, as it is for
// anybody at a served address.

import {launchBrowser, sleep, cleanup, browserPath} from "../../swem-host/tests/cdp_browser.mjs";
import {OWNER, addBotAndPair, messenger, pageInTheMessenger} from "./messenger_common.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, word, api, bigFile] = process.argv.slice(2);
const browser = browserPath();
if (!url || !word || !api || !bigFile) {
  console.error("usage: messenger_app_driver.mjs <address> <word> <bot-api-address> <a-big-file>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "messenger-app", door: true});
const tg = messenger(api);
const {written, sent, sentTo} = tg;
const page = pageInTheMessenger(b);
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

// --- Sign in, the agent, the bot --------------------------------------------
await b.holdPasskeys();
await says("the first start", "h1", "Make this Workbench yours");
await write("The word it printed", word.toUpperCase().replaceAll("-", " "));
await write("A name for this device", "Laptop");
if (!(await b.pressText("button", "Register this device"))) cleanup(1, "the device cannot be registered");
await says("the codes", "h1", "Codes to come back with");
await b.evaluate(`document.querySelector('input[type="checkbox"]').click()`);
if (!(await b.pressText("button", "Open the Workbench"))) cleanup(1, "the Workbench does not open");
await says("the Workbench", 'nav[aria-label="Workbench"]', "Settings");
const profile = await b.makeAgent("hands");
await addBotAndPair(b, api, profile, {guests: "nobody", tg});
await written(OWNER, "hello from my phone");
await b.waitFor("the agent answers", async () => sentTo(await sent(), OWNER.id).some((call) => String(call.body.text).includes(USAGE)), 200, 200);
step("signed in, the bot added, a first word said to it");

// --- The bot offers its page with one button, at this address ----------------
await written(OWNER, "/app");
await b.waitFor("the bot offers its page", async () =>
  sentTo(await sent(), OWNER.id).some((call) => call.body.reply_markup?.inline_keyboard?.flat().some((one) => one.web_app?.url)), 100);
const offered = sentTo(await sent(), OWNER.id).flatMap((call) => call.body.reply_markup?.inline_keyboard?.flat() ?? []).find((one) => one.web_app?.url);
const appUrl = offered.web_app.url;
const opened = new URL(appUrl);
if (opened.origin !== new URL(url).origin) cleanup(1, `the button does not carry this Workbench's address: ${appUrl}`);
if (opened.searchParams.get("open") !== "files" || !opened.searchParams.get("channel") || !opened.searchParams.get("signed")) cleanup(1, `the button says too little: ${appUrl}`);
step(`the bot offers its page at this address: ${offered.text}`);

// --- From the phone there is no passkey: the signature lets the owner in ---
// (The laptop's own session is ended first; the phone's browser never had it.)
await b.ask("/api/access/sign-out", {method: "POST", headers: {"content-type": "application/json"}, body: "{}"});
await page.open(appUrl, OWNER);
await b.waitFor("the page is drawn for the owner from the messenger", async () => b.exists('[data-in-messenger="owner"]'), 150);
if (await b.exists(".w-rail")) cleanup(1, "the rail is drawn inside the messenger");
await b.waitFor("the agent's tabs", async () => (await page.tabs()).length > 0, 150);
await b.waitFor("the Files tab, as the bot pointed", async () => (await page.activeTab()) === "Files", 100);
if (!(await b.pressText('[data-in-messenger] .k-tab', "Chat"))) cleanup(1, "no Chat tab");
await b.waitFor("the chat as it stands", async () => page.streamSays("hello from my phone"), 150);
step("opened from the bot, the page knows who you are and draws your agent");

// --- A file bigger than the messenger takes, to the agent --------------------
const placed = await b.setFileInputFiles('.w-composer input[type="file"]', [bigFile]);
if (!placed) cleanup(1, "the composer takes no file");
await b.fill(".w-composer-input", "a big file for you");
await b.waitFor("Send", async () => b.evaluate(`!document.querySelector('.w-composer button[type="submit"]')?.disabled`));
await b.click('.w-composer button[type="submit"]');
await b.waitFor("the words are in the chat", async () => page.streamSays("a big file for you"), 300, 200);
await b.waitFor("the agent answered what came with the file", async () =>
  sentTo(await sent(), OWNER.id).filter((call) => String(call.body.text).includes(USAGE)).length >= 2, 300, 200);
step("a big file went to the agent through the page");

// --- What the agent put out is here to read ---------------------------------
await written(OWNER, JSON.stringify({write: "outbox/report.txt", text: "the report, for the phone"}));
if (!(await b.pressText('[data-in-messenger] .k-tab', "Files"))) cleanup(1, "no Files tab");
await b.waitFor("what the agent put out is listed", async () =>
  (await b.evaluate(`document.querySelector('[data-agent-panel="files"]')?.innerText ?? ""`)).includes("report.txt"), 200, 200);
step("what the agent put out is listed on the Files tab");

// --- Somebody with a signature that is not the messenger's ------------------
await b.ask("/api/access/sign-out", {method: "POST", headers: {"content-type": "application/json"}, body: "{}"});
await page.open(appUrl, OWNER, "000000:not-the-bots-token");
await b.waitFor("nobody is let in: the door", async () => b.exists('[aria-label="Sign in"]'), 100);
if (await b.exists("[data-in-messenger]")) cleanup(1, "a signature that is not the messenger's let somebody in");
step("a signature that is not the messenger's lets nobody in");

console.log("messenger app OK");
await sleep(200);
await b.close();
process.exit(0);
