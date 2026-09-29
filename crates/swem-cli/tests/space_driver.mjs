// A person adds a catalog in the Store, installs a server from it that
// declares a home App, and finds that server on the space switcher beside
// the host's own spaces: choosing it mounts the server's App.
//
// usage: node space_driver.mjs <url> <catalog-url>
import {launchBrowser, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, catalogUrl] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!url || !catalogUrl) {
  console.error("usage: space_driver.mjs <url> <catalog-url>");
  process.exit(2);
}
const b = await launchBrowser({browser, url, label: "space"});
const setValue = (id, value) => b.evaluate(`(() => {
  const field = document.getElementById(${JSON.stringify(id)});
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(field, ${JSON.stringify(value)});
  field.dispatchEvent(new Event("input", {bubbles: true}));
})()`);

const spaces = async () => ((await b.ask("/api/spaces")).body ?? []).map((space) => space.server);
if ((await spaces()).includes("notes")) cleanup(1, "an App is offered before its server was installed");
await b.goTo("#/store");
await b.waitFor("the Store", async () => b.exists("#index-url"), 100);
await setValue("index-url", catalogUrl);
await b.click("#index-add");
await b.waitFor("the catalog's server", async () => b.exists('.store-entry[data-kind="server"][data-id="notes"]'), 300);
step("the catalog lists the server");

await b.click('.store-install[data-kind="server"][data-id="notes"]');
await b.consent();
await b.waitFor("the server installed", async () =>
  b.exists('.store-entry[data-kind="server"][data-id="notes"][data-installed="true"]'), 600, 500);
step("the server is installed from the Store");

// The rail learns of the App while the page is open: no reload.
let named = "";
await b.waitFor("the server's App on the rail", async () => {
  const space = ((await b.ask("/api/spaces")).body ?? []).find((one) => one.server === "notes");
  named = space?.name ?? "";
  return named !== "" && b.evaluate(`[...document.querySelectorAll('nav[aria-label="Workbench"] .k-rail-item')].some((one) => one.innerText.trim() === ${JSON.stringify(named)})`);
}, 120, 500);
step(`the installed server is offered on the rail (${named})`);
if (!(await b.pressText('nav[aria-label="Workbench"] .k-rail-item', named))) cleanup(1, "the rail's row cannot be pressed");
await b.waitFor("the App in the main area", async () => b.exists('.server-space[data-server="notes"]'));
await b.waitFor("the App mounted", async () => b.evaluate(`!!document.querySelector('.server-space .domain-app iframe')`), 300);
await b.waitFor("the App to settle", async () =>
  (await b.evaluate(`document.querySelector('.server-space .app-status')?.textContent ?? ""`)) === "", 300);
step("the server's App is open in its space");

// A person goes elsewhere and comes back: the App is the one they left, not
// another one opened in its place.
await b.evaluate(`document.querySelector('.server-space[data-server="notes"] iframe').dataset.left = "here"`);
await b.goTo("#/store");
await b.waitFor("the Store in its place", async () =>
  b.evaluate(`document.querySelector('.server-space[data-server="notes"]')?.hidden === true && document.getElementById("index-url") !== null`), 100);
await b.goTo("#/apps/notes");
await b.waitFor("the space again", async () =>
  b.evaluate(`document.querySelector('.server-space[data-server="notes"]')?.hidden === false`), 100);
if ((await b.evaluate(`document.querySelector('.server-space[data-server="notes"] iframe')?.dataset.left ?? ""`)) !== "here") {
  cleanup(1, "the App was opened anew when the person came back to its space");
}
step("the App is the one the person left");
console.log("space OK");
cleanup(0);
