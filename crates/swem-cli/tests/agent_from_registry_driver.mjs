// An agent this build never heard of, installed from the registry by its id
// on the command line, is offered by the product as one this computer has.
//
// The person makes it theirs, starts a session and gets an answer. Every
// step is a real click on the running product; the product's own list is
// what says the agent is available.

import {launchBrowser, sleep, cleanup, browserPath} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, agentId] = process.argv.slice(2);
const browser = browserPath();
if (!url || !agentId) {
  console.error("usage: agent_from_registry_driver.mjs <url> <agent-id>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "agent-from-registry"});

step("the product opens");

// Offered as this computer's - to be made an agent of, not to be installed -
// although no catalogue entry of this build names it: the receipt under the
// install root is what says so.
const offered = (await b.ask("/api/onboarding")).body?.agents?.find((one) => one.agent_id === agentId);
if (!offered?.available) {
  cleanup(1, `the installed agent ${agentId} is not offered as this computer's: ${JSON.stringify(offered)}`);
}
step(`${agentId} is offered as installed`);

await b.makeAgent(agentId);
step("an agent stands on it");

const said = "hello from a person whose agent the product never heard of";
const conversation = await b.say(said);
if (!conversation.some((text) => text.includes(said))) {
  cleanup(1, `what the person said is not in the chat: ${JSON.stringify(conversation)}`);
}
if (conversation.length < 2) {
  cleanup(1, `the agent said nothing back: ${JSON.stringify(conversation)}`);
}
const outcome = "answered";
step("the agent answered");
await sleep(200);

console.log("registry agent OK");
console.log(JSON.stringify({outcome, conversation}));
cleanup(0, "agent from registry done");
