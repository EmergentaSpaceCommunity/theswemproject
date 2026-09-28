// A form and a link an agent asks for, answered in its chat.
//
// An engine may ask before there is a session at all, and again in a turn.
// What it asks is drawn in the chat, held to the schema by the fields
// themselves, and a link is opened only by the person saying yes to it.

import {launchBrowser, cleanup, sleep} from "./cdp_browser.mjs";

const [browser, url] = process.argv.slice(2);
if (!browser || !url) {
  console.error("usage: workbench_shell_elicitation_cdp_driver.mjs <browser> <url>");
  process.exit(2);
}
const b = await launchBrowser({browser, url, label: "elicitation"});

const profile = ((await b.ask("/api/profiles")).body ?? [])[0]?.profile_id;
if (!profile) cleanup(2, "the fixture has no agent");
await b.openAgent(profile);
const form = () => b.evaluate(`document.querySelector("form.w-ask")?.innerText ?? ""`);
const asks = async (what) => {
  await b.waitFor(what, async () => b.exists("form.w-ask button[type=submit]:not(:disabled)"), 200);
  return form();
};
const set = (name, value) => b.evaluate(`(() => {
  const field = document.querySelector('form.w-ask [data-name=${JSON.stringify(name)}]');
  if (field instanceof HTMLSelectElement && field.multiple) {
    for (const option of field.options) option.selected = ${JSON.stringify(value)}.includes(option.value);
  } else {
    field.value = ${JSON.stringify(value)};
  }
})()`);
const answered = async (messages) => b.waitFor("the turn ends", async () => {
  const chat = await b.chat();
  return chat !== null && chat.deliveries.length === 0 && chat.messages.length >= messages;
}, 300);

// Asked before there is a session: the first thing said starts the engine,
// and the engine asks where it works before it answers anything.
await b.say(JSON.stringify({fixture: "elicitation-form-v0.1"}), {answered: false});
const initializeContext = await asks("what the engine asks before its session");
await set("workspace_label", "browser-pre-session");
await b.click("form.w-ask button[type=submit]");

// And in the turn: the form the message asked for.
await b.waitFor("the form of the turn", async () => b.exists('form.w-ask [data-name="strategy"]'), 300);
const formContext = await asks("the form");
// An answer that leaves out a choice the schema requires cannot be sent. The
// several-choice field carries the schema's requirement as the browser's
// own, so pressing Answer with nothing chosen does not send the form.
await set("strategy", "bold");
await set("iterations", "3");
await set("stems", []);
const emptyChoice = JSON.parse(await b.evaluate(`JSON.stringify((() => {
  const stems = document.querySelector('form.w-ask [data-name="stems"]');
  return {required: stems.required, valid: stems.checkValidity(), selected: stems.selectedOptions.length};
})())`));
if (!emptyChoice.required || emptyChoice.valid || emptyChoice.selected !== 0) {
  cleanup(1, `the several-choice field does not hold a person to the schema: ${JSON.stringify(emptyChoice)}`);
}
await b.click("form.w-ask button[type=submit]");
await sleep(500);
if (!(await b.exists('form.w-ask [data-name="strategy"]'))) cleanup(1, "an answer without a required choice closed the form");
if (((await b.chat())?.questions ?? []).length !== 1) cleanup(1, "an answer without a required choice was sent");

await set("strategy", "bold");
await set("iterations", "3");
await set("gain_db", "-1.25");
await set("normalize", "true");
await set("stems", ["voice", "fx"]);
await b.click("form.w-ask button[type=submit]");
await answered(2);

// A link: said where it leads, and opened by saying yes.
await b.say(JSON.stringify({fixture: "elicitation-url-v0.1"}), {answered: false});
await b.waitFor("the link", async () => (await form()).includes("example.invalid"), 300);
const urlContext = await form();
const pages = async () => (await b.send("Target.getTargets")).targetInfos.filter((entry) => entry.type === "page");
const before = (await pages()).length;
if ((await pages()).some((entry) => entry.url.startsWith("https://example.invalid/connect"))) {
  cleanup(2, "the link was opened before anybody said yes");
}
await b.click("form.w-ask button[type=submit]");
await b.waitFor("the link opened", async () =>
  (await pages()).some((entry) => entry.url.startsWith("https://example.invalid/connect")), 100);
const after = (await pages()).length;
await answered(4);

const chat = await b.chat();
console.log(JSON.stringify({
  chat: chat.chat.chat_id,
  agent: chat.chat.members.find((member) => member.kind === "agent")?.participant_id,
  initialize_context: initializeContext,
  form_context: formContext,
  url_context: urlContext,
  pages_before: before,
  pages_after: after,
}));
cleanup(0);
