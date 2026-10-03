// A Workbench on a laptop: no address from outside. The person installs a
// tunnel from the Store, sends /app to the bot, and the bot's button opens
// the Workbench's page inside the messenger through the tunnel - which here
// is a fixture that stands at the Workbench's own gate, so the walk really
// goes through the gate. What the gate answers is the page and nothing
// else. Closed from the Channels page, the page says to send /app again.

import {createHmac} from "node:crypto";
import {launchBrowser, sleep, cleanup, browserPath} from "../../swem-host/tests/cdp_browser.mjs";
import {OWNER, TOKEN, addBotAndPair, messenger} from "./messenger_common.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, api, catalogUrl, bigFile, hosted] = process.argv.slice(2);
const browser = browserPath();
if (!url || !api || !catalogUrl || !bigFile || !hosted) {
  console.error("usage: messenger_tunnel_driver.mjs <address> <bot-api-address> <catalog> <a-big-file> <where-the-page-is-hosted>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "messenger-tunnel"});
const origin = new URL(url).origin;
const tg = messenger(api);
const {written, sent, sentTo} = tg;
const USAGE = "this fixture takes";
const signed = (user, token = TOKEN) => {
  const fields = {auth_date: String(Math.floor(Date.now() / 1000)), user: JSON.stringify(user)};
  const check = Object.keys(fields).sort().map((name) => `${name}=${fields[name]}`).join("\n");
  const secret = createHmac("sha256", "WebAppData").update(token).digest();
  const hash = createHmac("sha256", secret).update(check).digest("hex");
  return new URLSearchParams({...fields, hash}).toString();
};
const setValue = async (id, value) =>
  b.evaluate(`(() => {
    const field = document.getElementById(${JSON.stringify(id)});
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(field, ${JSON.stringify(value)});
    field.dispatchEvent(new Event('input', {bubbles: true}));
  })()`);
const open = async (where) => {
  await b.send("Page.navigate", {url: where});
  await sleep(600);
};
const buttons = async () => sentTo(await sent(), OWNER.id).flatMap((call) => call.body.reply_markup?.inline_keyboard?.flat() ?? []).filter((one) => one.web_app?.url);

step("the product opens");
const profile = await b.makeAgent("hands");
const owner = (await b.ask("/api/people")).body?.owner?.name ?? "";
await addBotAndPair(b, api, profile, {guests: "nobody", appAt: hosted, tg});
await written(OWNER, "hello from my phone");
await b.waitFor("the agent answers", async () => sentTo(await sent(), OWNER.id).some((call) => String(call.body.text).includes(USAGE)), 200, 200);

// --- No address: the page says so, and the bot says what is missing ---------
// (The tunnel that came with the product is beside the binary, but the tool
// it drives is not installed here, so it cannot open one either.)
await b.goTo("#/providers/channels");
await b.waitFor("the row from outside", async () => b.exists('#reach-row[data-reach-state="none"]'), 100);
const noWay = await b.evaluate(`document.getElementById("reach-row")?.innerText ?? ""`);
if (!noWay.includes("No address from outside")) cleanup(1, `the row does not say so: ${noWay}`);
await written(OWNER, "/app");
await b.waitFor("the bot says what is missing", async () =>
  sentTo(await sent(), OWNER.id).some((call) => String(call.body.text).includes("address from outside")), 150);
if ((await buttons()).length > 0) cleanup(1, "a button was sent with no address to open");
step("with no address and no tunnel, the page and the bot say what to do");

// --- A tunnel from the Store ----------------------------------------------------
await b.goTo("#/store");
await b.waitFor("the Store", async () => b.exists("#index-url"), 100);
await setValue("index-url", catalogUrl);
await b.click("#index-add");
await b.waitFor("the tunnel listed", async () => b.exists('.store-entry[data-kind="swem/tunnel@1"][data-id="nowhere"]'), 300);
await b.click('.store-install[data-kind="swem/tunnel@1"][data-id="nowhere"]');
const question = await b.consent();
if (!/^Install /.test(question)) cleanup(1, `the consent question: ${JSON.stringify(question)}`);
await b.waitFor("the tunnel installed", async () => b.exists('.store-entry[data-kind="swem/tunnel@1"][data-id="nowhere"][data-installed="true"]'), 600, 500);
step("a tunnel is installed from the Store");

// --- /app opens the tunnel and sends the button ----------------------------
await written(OWNER, "/app");
await b.waitFor("the bot offers its page through the tunnel", async () => (await buttons()).length > 0, 200, 200);
const offered = (await buttons())[0];
const opened = new URL(offered.web_app.url);
const at = opened.searchParams.get("at");
if (!at || !at.startsWith("http://127.0.0.1:")) cleanup(1, `the button does not say where the Workbench is reached: ${offered.web_app.url}`);
if (at === origin) cleanup(1, "the button points at the Workbench itself, not at the gate");
await b.goTo("#/providers/channels");
await b.waitFor("the row says the tunnel is open", async () => b.exists('#reach-row[data-reach-state="tunnel"]'), 100);
const openRow = await b.evaluate(`document.getElementById("reach-row")?.innerText ?? ""`);
if (!openRow.includes("nowhere") || !openRow.includes(at)) cleanup(1, `the row does not say through what and where: ${openRow}`);
step("/app opened the tunnel and the bot sent a button pointing through it");

// --- The page, through the gate ---------------------------------------------
await open(`${offered.web_app.url}#tgWebAppData=${encodeURIComponent(signed(OWNER))}`);
await b.waitFor("the page knows who opened it", async () =>
  (await b.evaluate(`document.getElementById("you")?.innerText ?? ""`)).includes(`You are ${owner} here`), 100);
const placed = await b.setFileInputFiles("#file", [bigFile]);
if (!placed) cleanup(1, "the file could not be put in the page's input");
await b.fill("#words", "through the tunnel");
await b.click("#upload");
await b.waitFor("the file is sent through the gate", async () => b.exists("#sending[data-sent]"), 300, 200);
await b.waitFor("the agent answered what came with the file", async () =>
  sentTo(await sent(), OWNER.id).filter((call) => String(call.body.text).includes(USAGE)).length >= 2, 200, 200);
// And the gate answers nothing but the page - asked from here, as a
// stranger at the public address would ask.
const refused = [];
for (const path of ["/api/chats", "/", "/api/access", "/api/channels/x/receive", "/api/reach"]) {
  const response = await fetch(at + path, {method: path.endsWith("receive") ? "POST" : "GET"});
  refused.push(`${path}:${response.status}`);
}
if (refused.some((one) => !one.endsWith(":404"))) cleanup(1, `the gate answers more than the page: ${refused.join(" ")}`);
step("the page works through the gate, which answers nothing else");

// --- Closed from the Channels page, the page says what to do ---------------
await open(`${origin}/#/providers/channels`);
await b.waitFor("the row with the tunnel", async () => b.exists("#tunnel-close"), 150);
await b.click("#tunnel-close");
await b.waitFor("the tunnel closed", async () => b.exists('#reach-row[data-reach-state="none"]'), 100);
await open(`${offered.web_app.url}#tgWebAppData=${encodeURIComponent(signed(OWNER))}`);
await b.waitFor("the page says to send /app again", async () =>
  (await b.evaluate(`document.getElementById("why")?.innerText ?? ""`)).includes("Send /app to the bot"), 150, 200);
step("closed, the page says to send /app to the bot again");

console.log("messenger tunnel OK");
await sleep(200);
await b.close();
process.exit(0);
