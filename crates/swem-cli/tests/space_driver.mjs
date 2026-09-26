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

await b.waitFor("the shell", async () => b.exists("#space-store"));
if (await b.exists("#space-server-notes")) cleanup(1, "a space is offered before its server was installed");
await b.click("#space-store");
await setValue("index-url", catalogUrl);
await b.click("#index-add");
await b.waitFor("the catalog's server", async () => b.exists('.store-entry[data-kind="server"][data-id="notes"]'), 300);
step("the catalog lists the server");

const before = b.dialogs.length;
await b.click('.store-install[data-kind="server"][data-id="notes"]');
await b.waitFor("the consent question", async () => b.dialogs.length > before, 200);
await b.waitFor("the server installed", async () =>
  b.exists('.store-entry[data-kind="server"][data-id="notes"][data-installed="true"]'), 600, 500);
step("the server is installed from the Store");

// The switcher learns of the space while the page is open: no reload.
await b.waitFor("the server's space on the switcher", async () => b.exists("#space-server-notes"), 120, 500);
step("the installed server is offered as a space");
await b.click("#space-server-notes");
await b.waitFor("the space in the main area", async () => b.exists('#server-space[data-server="notes"]'));
await b.waitFor("the App mounted", async () => b.evaluate(`!!document.querySelector('#server-space .domain-app iframe')`), 300);
await b.waitFor("the App to settle", async () =>
  (await b.evaluate(`document.querySelector('#server-space .app-status')?.textContent ?? ""`)) === "", 300);
step("the server's App is open in its space");
console.log("space OK");
cleanup(0);
