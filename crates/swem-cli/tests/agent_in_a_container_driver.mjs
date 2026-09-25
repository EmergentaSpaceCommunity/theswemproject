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
await b.waitFor("the shell", async () => b.exists("#space-agent"));
await b.click("#space-agent");
await b.waitFor("the agent offered", async () => b.exists("#agent-options .agent-option"));
await b.click('[data-agent-id="hands"]');
await b.waitFor("a profile exists", async () =>
  b.evaluate(`document.querySelectorAll("#profiles option").length > 0`), 200);
step("the agent is theirs");

// The choice itself: a row in the list of places this product can run an
// agent, picked the way the permission choice beside it is picked.
await b.click('[data-agent-tab="environment"]');
await b.waitFor("the environment panel", async () => b.exists('[data-agent-panel="environment"]:not([hidden])'));
// The list comes from the product, so it is not there the instant the panel
// is: a value set on a picker that is still empty or still disabled changes
// nothing and reads later as "the choice did not stick".
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
await b.click('[data-agent-tab="conversation"]');
await b.click("#open-new");
await b.waitFor("the session opens", async () =>
  b.evaluate(`(document.getElementById("route")||{textContent:"-"}).textContent !== "-"`), 600);
step("a session is open");

await setValue("prompt-text", JSON.stringify({write: `outbox/${wrote}`, text}), "HTMLTextAreaElement");
await b.click("#send");
await b.waitFor("the turn ends", async () =>
  b.evaluate(`(document.getElementById("turn-outcome")||{textContent:""}).textContent.startsWith("turn:")`), 600);
// The path the agent reports is the one it sees, which is the container's -
// not the person's. That is the whole point of the row it is running in.
const inside = (message) => message.includes(`/workspace/outbox/${wrote}`);
const said = await b.bodiesUntil((messages) => messages.some(inside));
if (!said.some(inside)) {
  cleanup(1, `the agent did not write inside the container: ${JSON.stringify(said)}`);
}
step("the agent wrote a file from inside the container");

// And the person finds it where their own files are, without knowing any of
// that: the Files panel reads the same directory the container was given.
await b.click('[data-agent-tab="files"]');
await b.waitFor("the Files panel", async () => b.exists('[data-agent-panel="files"]:not([hidden])'));
await b.waitFor("the file is listed", async () => {
  await b.click("#files-refresh");
  return b.exists(`[data-files-area="outbox"] [data-file-name=${JSON.stringify(wrote)}]`);
}, 30, 500);
step("the person's Files show what the container wrote");

// Ending the session is the other half of the claim: the container exists
// while the agent does and not a moment longer, and a person ends it the way
// they end any session.
// Disconnect rather than Close: closing is an ACP capability an agent has to
// advertise, and this one does not. Either way the person is finished with
// the session, which is what the container's life is tied to.
await b.click("#disconnect");
// The rail's own buttons say whether anything is connected, which is the
// page's answer to "is this session still open".
await b.waitFor("the session ends", async () =>
  b.evaluate(`document.getElementById("disconnect")?.disabled === true`), 60, 500);
step("the session is closed");

console.log("in a container OK");
console.log(JSON.stringify({says, said}));
cleanup(0, "agent in a container done");
