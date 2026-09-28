// A person puts their agent in a container, and then works with it.
//
// The choice is a row in a list, like the permission choice next to it, and
// nothing else about the page changes: the same session, the same composer,
// the same files. What changes is where the agent is - and the walk that runs
// this checks on the host's own disk that the file the agent wrote came out
// of the container into the person's workspace.

import {launchBrowser, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, environment, wrote, text] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!url || !environment || !wrote || !text) {
  console.error("usage: agent_in_a_container_driver.mjs <url> <environment id> <file> <text>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "agent-container"});

const setValue = async (id, value, kind = "HTMLInputElement") =>
  b.evaluate(`(() => {
    const field = document.getElementById(${JSON.stringify(id)});
    Object.getOwnPropertyDescriptor(window.${kind}.prototype, 'value').set.call(field, ${JSON.stringify(value)});
    field.dispatchEvent(new Event('input', {bubbles: true}));
  })()`);

step("the product opens");
const profile = await b.makeAgent("hands");
step("the agent is theirs");

// The choice itself: a row in the list of places this product can run an
// agent, picked the way the permission choice beside it is picked.
await b.openAgent(profile, "settings");
await b.waitFor("the environment panel", async () => b.exists('[data-agent-panel="environment"]:not([hidden])'));
// The list comes from the product, so it is not there the instant the panel
// is: a value set on a picker that is still empty or still disabled changes
// nothing and reads later as "the choice did not stick".
if (!(await b.pressText("#settings-together .k-row:has(#profile-environment) button", "Change"))) {
  cleanup(1, "where the agent lives cannot be changed");
}
await b.waitFor("the places this product can run an agent", async () =>
  b.evaluate(`(() => {
    const picker = document.getElementById("profile-environment-choice");
    return !!picker && !picker.disabled &&
      [...picker.options].some((option) => option.value === ${JSON.stringify(environment)});
  })()`), 100);

// Through the native setter, because React keeps its own idea of what this
// select holds and swallows a change event whose value it thinks it already
// has - the same reason the composer's text is set this way.
await setValue("profile-environment-choice", environment, "HTMLSelectElement");
await b.evaluate(`document.getElementById("profile-environment-choice").dispatchEvent(new Event("change", {bubbles: true}))`);
await b.waitFor("the profile says it runs there", async () =>
  (await b.evaluate(`document.getElementById("profile-environment")?.dataset.environment || ""`)) === environment,
  100);
const says = await b.evaluate(
  `(document.getElementById("profile-environment")||{textContent:""}).textContent`);
step(`the agent runs ${says}`);

// Everything after this is the ordinary page: nothing about holding a
// conversation changes because the agent is somewhere else.
await b.openAgent(profile);
// The path the agent reports is the one it sees, which is the container's -
// not the person's. That is the whole point of the row it is running in.
const inside = (message) => message.includes(`/workspace/outbox/${wrote}`);
const said = await b.say(JSON.stringify({write: `outbox/${wrote}`, text}));
if (!said.some(inside)) {
  cleanup(1, `the agent did not write inside the container: ${JSON.stringify(said)}`);
}
step("the agent wrote a file from inside the container");

// And the person finds it where their own files are, without knowing any of
// that: the Files panel reads the same directory the container was given.
await b.openAgent(profile, "files");
await b.waitFor("the Files panel", async () => b.exists('[data-agent-panel="files"]:not([hidden])'));
await b.waitFor("the file is listed", async () => {
  await b.click("#files-refresh");
  return b.exists(`[data-files-area="outbox"] [data-file-name=${JSON.stringify(wrote)}]`);
}, 30, 500);
step("the person's Files show what the container wrote");

// Putting the agent to sleep is the other half of the claim: the container
// exists while the agent is awake and not a moment longer.
const agent = await b.openAgent(profile);
if (!(await b.pressText(".w-head button", "Put to sleep"))) cleanup(1, "the agent's header has no way to put it to sleep");
const chat = await b.chat();
await b.waitFor("the agent sleeps", async () =>
  (await b.ask(`/api/chats/${chat.chat.chat_id}/agents/${agent}/session`)).body?.connection_id === null, 60, 500);
step("the agent is asleep");

console.log("in a container OK");
console.log(JSON.stringify({says, said}));
cleanup(0, "agent in a container done");
