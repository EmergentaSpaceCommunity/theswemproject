// A profile that says its agent runs somewhere this product does not have.
//
// The profile carries the name of an environment, and until the catalogue
// existed nothing read it: whatever it said, the product started a process on
// this machine. This walk takes a profile whose environment this product does
// not offer and checks both halves of the fix — the page says so in words a
// person can act on, and starting a session is refused with the name in the
// message rather than quietly run here.

import {launchBrowser, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, profileId, environmentId] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!url || !profileId || !environmentId) {
  console.error("usage: agent_unknown_environment_driver.mjs <url> <profile> <environment>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "unknown-environment"});

step("the product opens");
await b.waitFor("the shell", async () => b.exists("#space-agent"));
await b.click("#space-agent");
await b.waitFor("the profiles", async () =>
  b.evaluate(`document.querySelectorAll("#profiles option").length > 0`), 300);
await b.evaluate(`(() => {
  const picker = document.getElementById("profiles");
  picker.value = ${JSON.stringify(profileId)};
  picker.dispatchEvent(new Event("change", {bubbles: true}));
})()`);
await b.waitFor("the profile is the one in question", async () =>
  (await b.evaluate(`document.getElementById("profiles")?.value`)) === profileId, 200);
step(`the profile that says it runs in ${environmentId}`);

// Half one: the page says it, and says it about this exact environment.
await b.click('[data-agent-tab="environment"]');
await b.waitFor("the environment panel", async () => b.exists('[data-agent-panel="environment"]:not([hidden])'));
await b.waitFor("the page names the environment", async () =>
  (await b.evaluate(`document.getElementById("profile-environment")?.dataset.environment || ""`)) === environmentId,
  200);
const said = await b.evaluate(`JSON.stringify({
  where: document.getElementById("profile-environment")?.textContent || "",
  why: document.getElementById("environment-summary")?.textContent || "",
})`).then(JSON.parse);
if (!said.where.includes(environmentId)) {
  cleanup(1, `the page does not say the environment is unknown: ${JSON.stringify(said)}`);
}
step("the page says this product does not have that environment");

// Half two, and the one that matters: the session is refused, not started on
// this machine instead.
await b.click('[data-agent-tab="conversation"]');
await b.click("#open-new");
await b.waitFor(
  "the product refuses the session",
  async () => {
    const opened = await b.evaluate(
      `(document.getElementById("route")||{textContent:"-"}).textContent !== "-" ? "opened" : null`);
    if (opened === "opened") cleanup(1, "the session opened despite an environment this product does not have");
    // `close-error` is this page's one problem line, whatever raised it.
    const status = await b.evaluate(`document.getElementById("close-error")?.textContent || ""`);
    return status.includes(environmentId) ? status : null;
  },
  120,
);
const refusal = await b.evaluate(`document.getElementById("close-error")?.textContent || ""`);
step(`refused, and the refusal names it (${refusal})`);

console.log("unknown environment OK");
console.log(JSON.stringify({said, refusal}));
cleanup(0, "unknown environment done");
