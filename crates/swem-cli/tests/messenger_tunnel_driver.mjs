// A Workbench on a laptop: no address from outside. The person installs a
// tunnel from the Store, sends /app to the bot, and the bot's button opens
// the Workbench's own page inside the messenger through the tunnel - which
// here is a fixture that stands at the Workbench's own gate, so the walk
// really goes through the gate. The page is the page, drawn for somebody
// who came through a messenger at a phone's width: the owner has the
// agent's files and the chat, and hands a file of any size; a guest has
// their chat and nothing else. The gate lets in a messenger session and
// nothing else. Closed from the Channels page, the bot takes its button
// back.

import {launchBrowser, sleep, cleanup, browserPath} from "../../swem-host/tests/cdp_browser.mjs";
import {OWNER, STRANGER, addBotAndPair, messenger, pageInTheMessenger} from "./messenger_common.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, api, catalogUrl, bigFile, serversUrl] = process.argv.slice(2);
const browser = browserPath();
if (!url || !api || !catalogUrl || !bigFile || !serversUrl) {
  console.error("usage: messenger_tunnel_driver.mjs <address> <bot-api-address> <catalog> <a-big-file> <servers-catalog>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "messenger-tunnel"});
const origin = new URL(url).origin;
const tg = messenger(api);
const {written, sent, sentTo} = tg;
const page = pageInTheMessenger(b);
const USAGE = "this fixture takes";
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
const buttons = async (who = OWNER) => sentTo(await sent(), who.id).flatMap((call) => call.body.reply_markup?.inline_keyboard?.flat() ?? []).filter((one) => one.web_app?.url);

step("the product opens");
const profile = await b.makeAgent("hands");
// A server whose tool brings an App, from the Store, attached before the
// agent's first session: what an agent attaches it is told when a session
// starts.
await b.goTo("#/store");
await b.waitFor("the Store", async () => b.exists("#index-url"), 100);
await setValue("index-url", serversUrl);
await b.waitFor("the Store takes a catalog", async () => b.evaluate(`!document.getElementById("index-add")?.disabled`), 150);
await b.click("#index-add");
await b.waitFor("the server listed", async () => b.exists('.store-entry[data-kind="server"][data-id="notes"]'), 300);
await b.waitFor("the server installable", async () =>
  b.evaluate(`(() => { const one = document.querySelector('.store-install[data-kind="server"][data-id="notes"]'); return !!one && !one.disabled; })()`), 100);
await b.click('.store-install[data-kind="server"][data-id="notes"]');
await b.consent();
await b.waitFor("the server installed", async () => b.exists('.store-entry[data-kind="server"][data-id="notes"][data-installed="true"]'), 600, 500);
// And its twin that declares nothing of where its App works.
await b.waitFor("the desk server installable", async () =>
  b.evaluate(`(() => { const one = document.querySelector('.store-install[data-kind="server"][data-id="notes-desk"]'); return !!one && !one.disabled; })()`), 100);
await b.click('.store-install[data-kind="server"][data-id="notes-desk"]');
await b.consent();
await b.waitFor("the desk server installed", async () => b.exists('.store-entry[data-kind="server"][data-id="notes-desk"][data-installed="true"]'), 600, 500);
await b.openAgent(profile, "settings");
for (const server of ["notes", "notes-desk"]) {
  await b.waitFor(`the server ${server} of the agent`, async () => b.exists(`.server-attach[data-server="${server}"]`), 150);
  await b.click(`.server-attach[data-server="${server}"]`);
  await b.waitFor(`the server ${server} attached`, async () => b.evaluate(`!!document.querySelector('.server-attach[data-server="${server}"]')?.checked`), 100);
}
step("a server whose tool brings an App is installed and attached to the agent");
await addBotAndPair(b, api, profile, {guests: "anyone", tg});
await written(OWNER, "hello from my phone");
await b.waitFor("the agent answers", async () => sentTo(await sent(), OWNER.id).some((call) => String(call.body.text).includes(USAGE)), 200, 200);

// --- No address: the page says so, and the bot says what is missing ---------
await b.goTo("#/providers/channels");
await b.waitFor("the row from outside", async () => b.exists('#reach-row[data-reach-state="none"]'), 100);
const noWay = await b.evaluate(`document.getElementById("reach-row")?.innerText ?? ""`);
if (!noWay.includes("No address from outside")) cleanup(1, `the row does not say so: ${noWay}`);
if (!(await b.evaluate(`document.getElementById("tunnel-open")?.innerText ?? ""`)).includes("and open")) cleanup(1, "the page does not offer to install what the tunnel needs and open it");
await written(OWNER, "/app");
await b.waitFor("the bot asks to install what the tunnel needs", async () =>
  sentTo(await sent(), OWNER.id).some((call) => String(call.body.text).includes("a tunnel needs")
    && JSON.stringify(call.body.reply_markup ?? {}).includes("Install and open")), 150);
if ((await buttons()).length > 0) cleanup(1, "a button was sent with no address to open");
step("with no address and no tunnel, the page and the bot say what to do");

// --- A tunnel from the Store, and a server whose tool brings an App --------
await b.goTo("#/store");
await b.waitFor("the Store", async () => b.exists("#index-url"), 100);
await setValue("index-url", catalogUrl);
await b.waitFor("the Store takes a catalog", async () => b.evaluate(`!document.getElementById("index-add")?.disabled`), 150);
await b.click("#index-add");
await b.waitFor("the tunnel listed", async () => b.exists('.store-entry[data-kind="swem/tunnel@1"][data-id="nowhere"]'), 300);
await b.waitFor("the tunnel installable", async () =>
  b.evaluate(`(() => { const one = document.querySelector('.store-install[data-kind="swem/tunnel@1"][data-id="nowhere"]'); return !!one && !one.disabled; })()`), 100);
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
const gate = opened.origin;
if (!gate.startsWith("http://127.0.0.1:")) cleanup(1, `the button does not point at the gate: ${offered.web_app.url}`);
if (gate === origin) cleanup(1, "the button points at the Workbench itself, not at the gate");
if (opened.searchParams.get("open") !== "files" || !opened.searchParams.get("channel")) cleanup(1, `the button does not say what to open: ${offered.web_app.url}`);
await b.goTo("#/providers/channels");
await b.waitFor("the row says the tunnel is open", async () => b.exists('#reach-row[data-reach-state="tunnel"]'), 100);
const openRow = await b.evaluate(`document.getElementById("reach-row")?.innerText ?? ""`);
if (!openRow.includes("nowhere") || !openRow.includes(gate)) cleanup(1, `the row does not say through what and where: ${openRow}`);
step("/app opened the tunnel and the bot sent a button pointing through it");

// --- The page, through the gate, at a phone's width ------------------------
await page.open(offered.web_app.url, OWNER);
await b.waitFor("the page is drawn for the owner from the messenger", async () => b.exists('[data-in-messenger="owner"]'), 150);
if (await b.exists(".w-rail")) cleanup(1, "the rail is drawn inside the messenger");
const tabs = await page.tabs();
if (tabs.join(",") !== "Chat,Files,Apps") cleanup(1, `the owner's tabs: ${tabs.join(",")}`);
await b.waitFor("the Files tab, as the bot pointed", async () => (await page.activeTab()) === "Files", 100);
await b.waitFor("the agent's files", async () => b.exists('[data-agent-panel="files"]:not([hidden]) .w-tree-items'), 150);
if (!(await b.pressText('[data-in-messenger] .k-tab', "Chat"))) cleanup(1, "no Chat tab");
await b.waitFor("the chat as it stands", async () => page.streamSays("hello from my phone"), 150);
const placed = await b.setFileInputFiles('.w-composer input[type="file"]', [bigFile]);
if (!placed) cleanup(1, "the composer takes no file");
await b.fill(".w-composer-input", "through the tunnel");
await b.waitFor("Send", async () => b.evaluate(`!document.querySelector('.w-composer button[type="submit"]')?.disabled`));
await b.click('.w-composer button[type="submit"]');
await b.waitFor("the words are in the chat", async () => page.streamSays("through the tunnel"), 300, 200);
await b.waitFor("the agent answered what came with the file", async () =>
  sentTo(await sent(), OWNER.id).filter((call) => String(call.body.text).includes(USAGE)).length >= 2, 300, 200);
// And the gate lets in nothing but a messenger session - asked from here
// as a stranger at the public address would ask, with no cookie.
const refused = [];
for (const [method, path] of [["GET", "/api/chats"], ["POST", "/api/channels/x/receive"], ["GET", "/api/reach"], ["POST", "/api/access/sign-in/begin"], ["GET", "/api/stream"]]) {
  const response = await fetch(gate + path, {method});
  refused.push(`${path}:${response.status}`);
}
if (refused.some((one) => !one.endsWith(":403"))) cleanup(1, `the gate answers more than it should: ${refused.join(" ")}`);
step("the owner's page works through the gate at a phone's width, and the gate refuses everybody else");

// --- An App a tool of the agent brought: the bot's button, the Apps tab ----
await page.laptop();
await written(OWNER, JSON.stringify({tool: "save_note", server: "notes", arguments: {nonce: "messenger-nonce-1", text: "a note from the phone"}}));
await b.waitFor("the bot offers the App the agent brought", async () =>
  sentTo(await sent(), OWNER.id).some((call) => String(call.body.text).includes("brought an App")
    && (call.body.reply_markup?.inline_keyboard?.flat() ?? []).some((one) => one.web_app?.url?.includes("open=apps"))), 300, 200);
const broughtButton = sentTo(await sent(), OWNER.id).flatMap((call) => call.body.reply_markup?.inline_keyboard?.flat() ?? []).find((one) => one.web_app?.url?.includes("open=apps"));
await page.open(broughtButton.web_app.url, OWNER);
await b.waitFor("the page is drawn for the owner from the messenger", async () => b.exists('[data-in-messenger="owner"]'), 150);
await b.waitFor("the Apps tab, as the bot pointed", async () => (await page.activeTab()) === "Apps", 100);
await b.waitFor("the App the tool brought, drawn through the tunnel's second address", async () =>
  (await b.exists('[data-in-messenger] .w-apps:not([hidden]) #app-frame'))
    && (await b.evaluate(`[...document.querySelectorAll('[data-in-messenger] .w-apps:not([hidden]) > .k-caption')].map((one) => one.textContent).join(" ")`)).includes("agent App result delivered"), 600, 300);
const frameOrigin = await b.evaluate(`(() => { try { return new URL(document.getElementById("app-frame").src).origin; } catch { return ""; } })()`);
if (!frameOrigin.startsWith("http://127.0.0.1:") || frameOrigin === gate || frameOrigin === origin) cleanup(1, `the App is not drawn at the tunnel's second address: ${frameOrigin}`);
// The person acknowledges the call from inside the App's own frame; the
// App says back where the host told it it is.
await sleep(800);
for (let attempt = 0; attempt < 6; attempt += 1) {
  await b.evaluateEverywhere(`(() => { const one = document.getElementById("observed-ack"); if (one && !one.disabled) one.click(); })()`);
  await sleep(500);
}
// Closed, the tab lists what there is: the App that declared nothing of
// where it works is the Workbench's, with no way to open it here.
if (!(await b.pressText('[data-in-messenger] .w-apps button', "Close it"))) cleanup(1, "the App cannot be closed");
await b.waitFor("the desk's App listed as the Workbench's", async () =>
  (await b.evaluate(`document.querySelector('[data-in-messenger] [data-apps-elsewhere]')?.innerText ?? ""`)).includes("notes-desk"), 100);
if (await b.evaluate(`[...document.querySelectorAll('[data-in-messenger] .w-apps .k-rail-item')].some((one) => one.innerText.includes("notes-desk"))`)) cleanup(1, "an App that declared nothing is offered on the phone");
step("an App the agent brought opens on the page's Apps tab, through the tunnel's second address; the other is listed as the Workbench's");

// --- A guest: their chat and nothing else ------------------------------------
await written(STRANGER, "hello, I am Bob");
await b.waitFor("the agent answers Bob", async () => sentTo(await sent(), STRANGER.id).some((call) => String(call.body.text).includes(USAGE)), 200, 200);
await written(STRANGER, "/app");
await b.waitFor("the bot offers Bob his page", async () => (await buttons(STRANGER)).length > 0, 200, 200);
const bobs = (await buttons(STRANGER))[0];
if (new URL(bobs.web_app.url).searchParams.get("open") !== "chat") cleanup(1, `a guest's button opens ${bobs.web_app.url}`);
await page.open(bobs.web_app.url, STRANGER);
await b.waitFor("the page is drawn for a guest", async () => b.exists('[data-in-messenger="guest"]'), 150);
const bobTabs = await page.tabs();
if (bobTabs.join(",") !== "Chat") cleanup(1, `a guest's tabs: ${bobTabs.join(",")}`);
await b.waitFor("Bob's own chat", async () => page.streamSays("hello, I am Bob"), 150);
if (await page.streamSays("hello from my phone")) cleanup(1, "a guest sees the owner's chat");
step("a guest's page is their chat and nothing else");

// --- Closed from the Channels page, the bot takes its buttons back ----------
await page.laptop();
await open(`${origin}/#/providers/channels`);
await b.waitFor("the row with the tunnel", async () => b.exists("#tunnel-close"), 150);
await b.click("#tunnel-close");
await b.waitFor("the tunnel closed", async () => b.exists('#reach-row[data-reach-state="none"]'), 100);
await b.waitFor("the bot took its buttons back", async () =>
  sentTo(await sent(), OWNER.id, "editMessageText").some((call) => String(call.body.text).includes("send /app again"))
    && sentTo(await sent(), STRANGER.id, "editMessageText").length > 0, 150);
step("closed, the bot took its buttons back and said to send /app again");

console.log("messenger tunnel OK");
await sleep(200);
await b.close();
process.exit(0);
