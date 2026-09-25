// An agent this build never heard of, installed from the registry by its id
// on the command line, is offered by the product as one this computer has.
//
// The person makes it theirs, starts a session and gets an answer. Every
// step is a real click on the running product; the product's own list is
// what says the agent is available.

import {launchBrowser, sleep, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, agentId] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!url || !agentId) {
  console.error("usage: agent_from_registry_driver.mjs <url> <agent-id>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "agent-from-registry"});

step("the product opens");
await b.waitFor("the shell", async () => b.exists("#space-agent"));
await b.click("#space-agent");
await b.waitFor("the list of agents", async () => b.exists("#agent-options .agent-option"));

// Offered as installed - "Make it mine", not "Install" - although no
// catalogue entry of this build names it: the receipt under the install
// root is what says so.
const offer = await b.evaluate(`(() => {
  const row = document.querySelector('[data-agent-id=${JSON.stringify(agentId)}]');
  return row ? row.textContent : "missing";
})()`);
if (offer !== "Make it mine") {
  cleanup(1, `the installed agent ${agentId} is not offered as this computer's, the button says ${JSON.stringify(offer)}`);
}
step(`${agentId} is offered as installed`);

await b.click(`[data-agent-id=${JSON.stringify(agentId)}]`);
await b.waitFor(
  "a profile of it",
  async () => b.evaluate(`(() => {
    const status = (document.getElementById("onboarding-status")||{textContent:""}).textContent;
    const working = /Reading|Installing|Creating/.test(status);
    return !working && (status.length > 0 || document.querySelectorAll("#profiles option").length > 0);
  })()`),
  300,
  1000,
);
const status = await b.evaluate(`(document.getElementById("onboarding-status")||{textContent:""}).textContent`);
if (status) cleanup(1, `the product refused to make a profile: ${status}`);
step("a profile exists");

await b.waitFor("the product offers to start a session", async () =>
  b.evaluate(`document.querySelector("#open-new")?.disabled === false`));
await b.click("#open-new");
await b.waitFor("the session opens", async () =>
  b.evaluate(`(document.getElementById("route")||{textContent:"-"}).textContent !== "-"
    && (document.getElementById("connection-id")||{textContent:"-"}).textContent !== "-"`), 300);
step("a session with it");

const said = "hello from a person whose agent the product never heard of";
await b.evaluate(`(() => {
  const field = document.getElementById("prompt-text");
  Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set.call(field, ${JSON.stringify(said)});
  field.dispatchEvent(new Event('input', {bubbles: true}));
})()`);
await b.click("#send");
await b.waitFor("the turn ends", async () =>
  b.evaluate(`(document.getElementById("turn-outcome")||{textContent:""}).textContent.startsWith("turn:")`), 300);
const conversation = await b.evaluate(
  `JSON.stringify([...document.querySelectorAll("#conversation .message .message-body")].map(node => node.textContent))`,
).then(JSON.parse);
if (!conversation.some((text) => text.includes(said))) {
  cleanup(1, `what the person said is not in the conversation: ${JSON.stringify(conversation)}`);
}
if (conversation.length < 2) {
  cleanup(1, `the agent said nothing back: ${JSON.stringify(conversation)}`);
}
const outcome = await b.evaluate(`(document.getElementById("turn-outcome")||{textContent:""}).textContent`);
step(`the agent answered (${outcome})`);
await sleep(200);

console.log("registry agent OK");
console.log(JSON.stringify({outcome, conversation}));
cleanup(0, "agent from registry done");
