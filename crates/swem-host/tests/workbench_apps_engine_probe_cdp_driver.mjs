// What a sandboxed App can use on this host, measured where a person meets
// it: in the panel beside an agent's chat.
//
// The probe App of the fixture server is opened from the panel, its one
// button is pressed by a real click inside the frame, and the walk waits
// until the App handed over what it measured. The facts themselves are read
// by the Rust test from the receipt the server wrote.
// usage: node workbench_apps_engine_probe_cdp_driver.mjs <browser> <url> [app uri]

import {launchBrowser, cleanup, sleep} from "./cdp_browser.mjs";

const [browser, url, appUri = "ui://apps-fixture/engine-probe"] = process.argv.slice(2);
if (!browser || !url) {
  console.error("usage: node workbench_apps_engine_probe_cdp_driver.mjs <browser> <url> [app uri]");
  process.exit(2);
}
const b = await launchBrowser({
  browser, url, label: "probe",
  extraArgs: [
    "--autoplay-policy=no-user-gesture-required",
    // A fake capture device answers getUserMedia without a prompt: the walk
    // measures the host's grant and the App's reach, not a person's click.
    "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream",
  ],
});

const panel = '.w-apps:not([hidden])';
const status = () => b.evaluate(`[...document.querySelectorAll('${panel} > .k-caption')].map((one) => one.textContent).join(" ")`);
const clickInApp = async (x, y) => {
  const rect = JSON.parse(await b.evaluate(`JSON.stringify(document.getElementById("app-frame").getBoundingClientRect())`));
  await b.clickAt(rect.left + x, rect.top + y);
};

const agent = await b.openAgent("apps-probe");
await b.say("hello");
const chat = (await b.chat()).chat.chat_id;
if (!(await b.pressText(".w-composer button", "Apps"))) cleanup(2, "the composer has no Apps");
const probe = `${panel} .k-rail-item[title=${JSON.stringify(appUri)}]`;
await b.waitFor("the probe App among what its servers bring", async () => b.exists(probe), 300);
await b.click(probe);
await b.waitFor("the App ready", async () =>
  (await b.exists("#app-frame")) && (await status()).includes("app ready"), 300);
await sleep(500);
// The probe button sits at (20,60) 200x36 inside the App.
await clickInApp(120, 78);
await b.waitFor("what it measured, handed over", async () =>
  ((await b.chat())?.events ?? []).some((event) => event.kind === "host/app_tool_call"), 300);
await sleep(300);
// The facts are kept by now; closing is ordinary tidying the Rust test
// does as well.
await b.pressText(`${panel} button`, "Close it");
await b.putToSleep(chat, agent);

console.log(JSON.stringify({chat, agent}));
cleanup(0);
