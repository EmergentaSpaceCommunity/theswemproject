// What the product keeps after a person closed their editor and opened the
// same conversation again.
//
// The claim is that there is one conversation and not two: the editor kept an
// id, handed it back, and everything said on both sides of the closing is one
// chat in the record. So this driver counts the chats the product keeps,
// checks the one it finds is the one the editor named, and reads what was
// said in it back through the product's own door.

import {launchBrowser, cleanup, browserPath} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, profile, kept, ...wanted] = process.argv.slice(2);
const browser = browserPath();
if (!url || !profile || !kept || wanted.length === 0) {
  console.error("usage: editor_door_resume_driver.mjs <url> <profile> <chat> <said>...");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "editor-resume"});

step("the product opens");

// One conversation. Before the editor could hand an id back, each launch
// opened a conversation of its own and the record grew one per launch.
const record = await b.evaluate(`(async () => {
  const chats = await (await fetch("/api/chats")).json();
  const chat = await (await fetch("/api/chats/" + encodeURIComponent(${JSON.stringify(kept)}))).json();
  return JSON.stringify({chats: chats.map((chat) => chat.chat_id), chat});
})()`).then(JSON.parse);
if (record.chats.length !== 1) {
  cleanup(1, `the product keeps ${record.chats.length} chats, not one: ${JSON.stringify(record.chats)}`);
}
if (record.chats[0] !== kept) {
  cleanup(1, `the product keeps ${record.chats[0]}, and the editor worked in ${kept}`);
}
step("the product keeps one chat, and it is the one the editor named");

// And both messages are in it: what was said before the editor closed, and
// what was said after it came back, each said by the person from an editor
// and each answered by the agent.
const agent = (record.chat.chat.members || []).find((member) => member.kind === "agent");
if (!agent || agent.profile_id !== profile) {
  cleanup(1, `the chat is not with ${profile}: ${JSON.stringify(record.chat.chat.members)}`);
}
const messages = record.chat.messages || [];
const written = messages.filter((message) => message.sender_id !== agent.participant_id);
for (const said of wanted) {
  if (!written.some((message) => message.text.includes(said))) {
    cleanup(1, `the chat does not carry "${said}": ${JSON.stringify(written)}`);
  }
}
if (!written.every((message) => message.channel === "editor")) {
  cleanup(1, `something in this chat was not said from an editor: ${JSON.stringify(written)}`);
}
if (messages.length !== written.length * 2) {
  cleanup(1, `not everything said was answered: ${JSON.stringify(messages)}`);
}
step(`both messages are in one chat (${written.length} of them, all from an editor, all answered)`);

console.log("editor resume OK");
console.log(JSON.stringify({chats: record.chats.length, chat: kept, said: written.length}));
cleanup(0, "editor resume done");
