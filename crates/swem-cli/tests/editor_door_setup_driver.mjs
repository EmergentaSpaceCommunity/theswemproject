// A person sets their agent up in the Workbench, before ever opening an editor.
//
// This walk's claim is about the editor door, and the profile it talks to has
// to be a real one: made the way a person makes it, in the page, not written
// into the inventory by the gate. So this driver does only that, and says
// which profile it made.

import {launchBrowser, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!url) {
  console.error("usage: editor_door_setup_driver.mjs <url>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "editor-setup"});

step("the product opens");

// The person's own agent, declared on this machine, made from the page.
const profile = await b.makeAgent("hands");
step(`the agent is theirs (${profile})`);

// And the page says how to reach this agent from an editor. Without it the
// door exists and nobody knows the words to open it.
await b.openAgent(profile, "settings");
await b.waitFor("the agent's settings", async () => b.exists('[data-agent-panel="environment"]:not([hidden])'));
const command = await b.evaluate(
  `(document.getElementById("editor-command")||{textContent:""}).textContent.trim()`);
if (command !== `swem acp --profile ${profile}`) {
  cleanup(1, `the page does not say how to reach this agent from an editor: ${JSON.stringify(command)}`);
}
step(`the page gives the editor's command (${command})`);

console.log("editor setup OK");
console.log(JSON.stringify({profile}));
cleanup(0, "editor setup done");
