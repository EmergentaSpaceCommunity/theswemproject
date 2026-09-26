// A server that declares a home App is a space: the switcher offers it beside
// the host's own three, and choosing it mounts the App in the main area.
// usage: node workbench_shell_spaces_cdp_driver.mjs <browser> <url>
import {launchBrowser, cleanup} from "./cdp_browser.mjs";

const [browser, url] = process.argv.slice(2);
if (!browser || !url) {
  console.error("usage: node workbench_shell_spaces_cdp_driver.mjs <browser> <url>");
  process.exit(2);
}
const b = await launchBrowser({browser, url, label: "spaces"});
const step = (s) => console.log("step: " + s);

await b.waitFor("the shell", async () => b.exists("#space-store"));
await b.waitFor("the server's space on the switcher", async () => b.exists('#space-server-notes'), 300);
const label = await b.evaluate(`document.getElementById("space-server-notes")?.textContent ?? ""`);
if (!label.includes("notes")) cleanup(2, `the space is not named after its server: ${label}`);
step("the switcher offers the server's space beside the host's own");
await b.click("#space-server-notes");
await b.waitFor("the space in the main area", async () => b.exists('#server-space[data-server="notes"]'));
await b.waitFor("the App mounted", async () => b.evaluate(`!!document.querySelector('#server-space .domain-app iframe')`), 300);
// The steps of opening are worth a line each; a ready App says nothing.
await b.waitFor("the App to settle", async () =>
  (await b.evaluate(`document.querySelector('#server-space .app-status')?.textContent ?? ""`)) === "", 300);
step("the App mounted in the space with nothing left to say");
if (await b.evaluate(`document.querySelector("#space-server-notes")?.getAttribute("aria-pressed")`) !== "true") {
  cleanup(2, "the switcher does not show the space as the one open");
}
console.log("spaces OK");
cleanup(0);
