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
await b.openAgent(profileId, "settings");
step(`the agent that says it runs in ${environmentId}`);

// Half one: the page says it, and says it about this exact environment.
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

// Half two, and the one that matters: what is said to it is not answered from
// this machine instead. The chat says why, and the reason names the
// environment.
await b.openAgent(profileId);
await b.say("are you there?", {answered: false});
let refusal = "";
await b.waitFor(
  "the product refuses to start it",
  async () => {
    const chat = await b.chat();
    if (!chat || chat.deliveries.length > 0) return null;
    if (chat.messages.length > 1) cleanup(1, "the agent answered despite an environment this product does not have");
    const ended = chat.events.filter((event) => event.kind === "chat/delivery").pop();
    refusal = ended?.payload?.outcome ?? "";
    return ended?.payload?.state === "failed" && refusal.includes(environmentId) ? refusal : null;
  },
  120,
);
await b.waitFor("the page says why", async () =>
  b.evaluate(`[...document.querySelectorAll(".k-notice.k-danger")].some((one) => one.textContent.includes(${JSON.stringify(environmentId)}))`), 50);
step(`refused, and the refusal names it (${refusal})`);

console.log("unknown environment OK");
console.log(JSON.stringify({said, refusal}));
cleanup(0, "unknown environment done");
