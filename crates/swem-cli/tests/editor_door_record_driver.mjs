// What the person sees in the page after they worked from their editor.
//
// The editor is a second door onto one product, not a second product: the
// conversation it held is in the same record the page reads, and the file the
// agent wrote is in the same Files panel. This driver looks for both, through
// the product's own door, the way a person would.

import {launchBrowser, cleanup, browserPath} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, profile, wrote, editor] = process.argv.slice(2);
const browser = browserPath();
if (!url || !profile || !wrote || !editor) {
  console.error("usage: editor_door_record_driver.mjs <url> <profile> <file> <editor name>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "editor-record"});

step("the product opens");

// The conversation the editor held is this person's chat, and the record
// says who said what in it and through what: the person, from their editor,
// by the name the editor gave at initialize.
const record = await b.evaluate(`(async () => {
  const chats = await (await fetch("/api/chats")).json();
  const people = await (await fetch("/api/people")).json();
  const chat = await (await fetch("/api/chats/" + encodeURIComponent(chats[0]?.chat_id))).json();
  return JSON.stringify({chats: chats.length, owner: people.owner, chat});
})()`).then(JSON.parse);
step(`the product keeps ${record.chats} chat(s)`);
const said = (record.chat.messages || [])[0];
if (said?.channel !== "editor" || said?.sender_id !== record.owner?.participant_id
    || !String(said?.channel_ref || "").endsWith(":" + editor)) {
  cleanup(1, `the record does not say the person said it from the editor: ${JSON.stringify(record.chat.messages)}`);
}
const answered = (record.chat.messages || [])[1];
const agent = (record.chat.chat.members || []).find((member) => member.kind === "agent");
if (!agent || agent.profile_id !== profile || answered?.sender_id !== agent.participant_id) {
  cleanup(1, `the answer is not the agent's: ${JSON.stringify(record.chat.messages)}`);
}
step(`the chat says who said what (${editor} through ${said.channel}, answered by @${agent.handle})`);

// The file that turn produced is in the person's own Files, because the agent
// worked in the profile's directory and not in the one the editor claimed.
await b.openAgent(profile, "files");
await b.waitFor("the Files panel", async () => b.exists('[data-agent-panel="files"]:not([hidden])'));
await b.waitFor("the editor's file is listed", async () => {
  await b.click("#files-refresh");
  return b.exists(`[data-files-area="outbox"] [data-file-name=${JSON.stringify(wrote)}]`);
}, 30, 500);
step("the file the editor's turn wrote is in the person's Files");

console.log("editor record OK");
console.log(JSON.stringify({chats: record.chats, chat: record.chat.chat.chat_id, through: said.channel}));
cleanup(0, "editor record done");
