// The Workbench's page inside the messenger: a bot of a served Workbench
// offers it with one button; opened from the bot, it knows who you are by
// the messenger's signature, takes a file of any size to the agent, shows
// what the agent put out, and the chat as it stands. Opened by nobody, it
// says so; opened with a signature that is not the messenger's, it is
// refused.
//
// Signed here the way the messenger signs: HMAC-SHA256 over the fields with
// a key derived from the bot's token, which the walk knows because the walk
// is the messenger.

import {createHmac} from "node:crypto";
import {launchBrowser, sleep, cleanup, browserPath} from "../../swem-host/tests/cdp_browser.mjs";
import {OWNER, TOKEN, addBotAndPair, messenger} from "./messenger_common.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, word, api, bigFile, hosted] = process.argv.slice(2);
const browser = browserPath();
if (!url || !word || !api || !bigFile || !hosted) {
  console.error("usage: messenger_app_driver.mjs <address> <word> <bot-api-address> <a-big-file> <where-the-page-is-hosted>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "messenger-app", door: true});
const tg = messenger(api);
const {written, sent, sentTo} = tg;
const USAGE = "this fixture takes";

/// What the messenger would hand the page: the person, signed for this bot.
const signed = (user, token = TOKEN) => {
  const fields = {auth_date: String(Math.floor(Date.now() / 1000)), user: JSON.stringify(user)};
  const check = Object.keys(fields).sort().map((name) => `${name}=${fields[name]}`).join("\n");
  const secret = createHmac("sha256", "WebAppData").update(token).digest();
  const hash = createHmac("sha256", secret).update(check).digest("hex");
  return new URLSearchParams({...fields, hash}).toString();
};
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
const open = async (where) => {
  await b.send("Page.navigate", {url: where});
  await sleep(600);
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
// You are who you are on the Workbench, not what the messenger calls you.
const owner = (await b.ask("/api/people")).body?.owner?.name ?? "";
if (!owner) cleanup(1, "the Workbench has no owner");
// The page is hosted elsewhere, as a copy of web/mini-app on any static host.
await addBotAndPair(b, api, profile, {guests: "nobody", appAt: hosted, tg});
await written(OWNER, "hello from my phone");
await b.waitFor("the agent answers", async () => sentTo(await sent(), OWNER.id).some((call) => String(call.body.text).includes(USAGE)), 200, 200);
step("signed in, the bot added, a first word said to it");

// --- The bot offers its page with one button -------------------------------
await written(OWNER, "/app");
await b.waitFor("the bot offers its page", async () =>
  sentTo(await sent(), OWNER.id).some((call) => call.body.reply_markup?.inline_keyboard?.flat().some((one) => one.web_app?.url)), 100);
const offered = sentTo(await sent(), OWNER.id).flatMap((call) => call.body.reply_markup?.inline_keyboard?.flat() ?? []).find((one) => one.web_app?.url);
const appUrl = offered.web_app.url;
const hostedAt = new URL(hosted);
if (!appUrl.startsWith(`${hostedAt.origin}${hostedAt.pathname}`)) cleanup(1, `the button does not open the hosted page: ${appUrl}`);
const opened = new URL(appUrl);
if (opened.searchParams.get("at") !== url.replace(/\/$/, "") || !opened.searchParams.get("channel")) cleanup(1, `the button does not say where the Workbench is: ${appUrl}`);
const channelId = opened.searchParams.get("channel");
step(`the bot offers its page, hosted elsewhere, pointing at this Workbench: ${offered.text}`);

// --- Opened by nobody, it says so ---------------------------------------------
await open(appUrl);
await b.waitFor("the page says it is opened from a bot", async () =>
  (await b.evaluate(`document.getElementById("why")?.innerText ?? ""`)).includes("opened from a bot"), 100);
step("opened by nobody, the page says what it is");

// --- Opened from the bot: who you are, the chat, nothing put out yet ----------
await open(`${appUrl}#tgWebAppData=${encodeURIComponent(signed(OWNER))}`);
await b.waitFor("the page knows who opened it", async () =>
  (await b.evaluate(`document.getElementById("you")?.innerText ?? ""`)).includes(`You are ${owner} here`), 100);
const title = await b.evaluate(`document.getElementById("title")?.innerText ?? ""`);
if (!title.includes("Fixture bot")) cleanup(1, `the page is not the bot's: ${title}`);
await b.waitFor("the chat so far", async () =>
  (await b.evaluate(`[...document.querySelectorAll("[data-message]")].map((one) => one.innerText).join("|")`)).includes("hello from my phone"), 100);
step("opened from the bot, the page knows who you are and shows the chat");

// --- A file bigger than the messenger takes, to the agent --------------------
const placed = await b.setFileInputFiles("#file", [bigFile]);
if (!placed) cleanup(1, "the file could not be put in the page's input");
await b.fill("#words", "a big file for you");
await b.click("#upload");
await b.waitFor("the file is sent", async () => b.exists("#sending[data-sent]"), 300, 200);
await b.waitFor("the words are in the chat", async () =>
  (await b.evaluate(`[...document.querySelectorAll("[data-message]")].map((one) => one.innerText).join("|")`)).includes("a big file for you"), 100);
await b.waitFor("the agent answered what came with the file", async () =>
  sentTo(await sent(), OWNER.id).filter((call) => String(call.body.text).includes(USAGE)).length >= 2, 200, 200);
step("a big file went to the agent through the page");

// --- What the agent put out is here to take ---------------------------------
await written(OWNER, JSON.stringify({write: "outbox/report.txt", text: "the report, for the phone"}));
await b.waitFor("what the agent put out is listed", async () => b.exists('[data-file="report.txt"]'), 200, 200);
const fetched = await b.evaluate(`(async () => {
  const hash = location.hash.slice(1);
  const data = new URLSearchParams(hash).get("tgWebAppData");
  const query = new URLSearchParams(location.search);
  const response = await fetch(query.get("at") + "/api/channels/" + query.get("channel") + "/app/files/report.txt", {headers: {"x-swem-app-data": data}});
  return response.ok ? await response.text() : "status " + response.status;
})()`);
if (fetched !== "the report, for the phone") cleanup(1, `the file was not served to the page: ${JSON.stringify(fetched)}`);
step("what the agent put out is taken through the page");

// --- Somebody with a signature that is not the messenger's ------------------
await open(`${appUrl}#tgWebAppData=${encodeURIComponent(signed(OWNER, "000000:not-the-bots-token"))}`);
await b.waitFor("the page is refused", async () => b.exists('#why[data-refused="403"]'), 100);
step("a signature that is not the messenger's is refused");

// --- The Workbench serves the same page itself, for a bot with no host ------
await open(`${url.replace(/\/$/, "")}/channels/${encodeURIComponent(channelId)}/app#tgWebAppData=${encodeURIComponent(signed(OWNER))}`);
await b.waitFor("the Workbench's own copy knows who opened it", async () =>
  (await b.evaluate(`document.getElementById("you")?.innerText ?? ""`)).includes(`You are ${owner} here`), 100);
step("the Workbench serves the page itself as well");

console.log("messenger app OK");
await sleep(200);
await b.close();
process.exit(0);
