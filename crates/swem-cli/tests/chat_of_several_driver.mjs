// A person puts two agents in one chat.
//
// They start the chat from the rail, write to it, pick who a message is
// for after an `@`, set after how many replies of agents to each other the
// chain waits, watch it wait and go on, and take one agent out.

import {launchBrowser, cleanup, browserPath} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url] = process.argv.slice(2);
const browser = browserPath();
if (!url) {
  console.error("usage: chat_of_several_driver.mjs <url>");
  process.exit(2);
}
const b = await launchBrowser({browser, url, label: "chat-of-several"});
const beside = 'aside[aria-label="About this chat"]';
const shownBeside = () => b.evaluate(`document.querySelector(${JSON.stringify(beside)})?.innerText ?? ""`);
const composer = () => b.evaluate(`document.querySelector(".w-composer-input")?.value ?? ""`);
const said = async () => {
  const chat = await b.chat();
  const people = (await b.ask("/api/people")).body.participants;
  return (chat?.messages ?? []).map((message) => ({
    by: people.find((one) => one.participant_id === message.sender_id)?.handle,
    text: message.text,
  }));
};
const settled = (least) => b.waitFor("the chat to settle", async () => {
  const chat = await b.chat();
  return chat !== null && chat.deliveries.length === 0 && chat.messages.length >= least;
}, 400);

step("the product opens");
await b.makeAgent("echo", "ada");
await b.makeAgent("echo", "bob");
step("two agents are theirs");

// A chat of both, from the rail.
if (!(await b.pressText('nav[aria-label="Workbench"] button', "New chat"))) cleanup(1, "the rail offers no chat of several");
await b.waitFor("the form of a new chat", async () => b.exists(".w-dialog input.k-field"));
await b.fill(".w-dialog input.k-field", "Release");
for (const name of ["ada", "bob"]) {
  if (!(await b.pressText(".w-dialog label", name))) cleanup(1, `${name} cannot be chosen for the chat`);
}
if (!(await b.pressText(".w-dialog button", "Start the chat"))) cleanup(1, "the chat cannot be started");
await b.waitFor("the chat", async () => (await b.evaluate("location.hash")).startsWith("#/chats/"), 100);
await b.waitFor("who is in it", async () => {
  const shown = await shownBeside();
  return shown.includes("@ada") && shown.includes("@bob");
}, 100);
const head = await b.evaluate(`document.querySelector(".w-head")?.innerText ?? ""`);
if (!head.includes("A chat of 3") || !head.includes("when someone names them")) cleanup(1, `the chat does not say what it is: ${head}`);
step("a chat of the person and two agents");

// What names nobody is for nobody, and the chat says so.
await b.say("good morning", {answered: false});
await b.waitFor("the chat to say nobody was named", async () =>
  (await b.evaluate(`document.querySelector(".w-composer .k-notice")?.innerText ?? ""`)).includes("named nobody"), 100);
if (((await b.chat())?.deliveries ?? []).length !== 0) cleanup(1, "what named nobody was given to somebody");
step("what names nobody is for nobody");

// Who it is for is picked after an @.
await b.fill(".w-composer-input", "@a");
await b.waitFor("who can be named", async () => b.exists('.w-composer [role="option"]'), 100);
const offered = await b.evaluate(`[...document.querySelectorAll('.w-composer [role="option"]')].map((one) => one.innerText.replace(/\\n/g, " ")).join(" / ")`);
if (!offered.includes("@ada") || offered.includes("@bob")) cleanup(1, `after "@a" the chat offers ${offered}`);
if (!(await b.pressText('.w-composer [role="option"]', "ada"))) cleanup(1, "a name cannot be picked");
await b.waitFor("the name in what is written", async () => (await composer()) === "@ada ", 100);
step("a name is picked after @");

await b.say("@ada say hello", {answered: false});
await settled(3);
const answered = (await said()).slice(2).map((one) => one.by);
if (answered.join(",") !== "ada") cleanup(1, `named ada, and ${JSON.stringify(answered)} answered`);
const names = await b.evaluate(`[...document.querySelectorAll(".w-mine .w-mention")].map((one) => one.innerText).join(",")`);
if (!names.includes("@ada")) cleanup(1, `the name does not stand out in what was said: ${JSON.stringify(names)}`);
step("the one named answered, and the other did not");

// After two replies of agents to each other the chain waits.
await b.evaluate(`(() => {
  const field = [...document.querySelectorAll(${JSON.stringify(`${beside} select`)})].at(-1);
  Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set.call(field, "2");
  field.dispatchEvent(new Event("change", {bubbles: true}));
})()`);
await b.waitFor("the limit taken", async () => (await b.chat())?.chat.reply_limit === 2, 100);
await b.say("@ada and @bob, agree on a name", {answered: false});
await b.waitFor("the chain to wait for a person", async () =>
  (await b.evaluate(`document.querySelector(".w-chain")?.innerText ?? ""`)).includes("waits for you"), 400);
await settled(4);
const held_at = (await b.chat()).chat.agent_replies;
const once = (await b.chat()).events.filter((event) => event.kind === "chat/held").length;
if (once !== 1) cleanup(1, `the chat said ${once} times that it waits`);
step(`agents answered each other ${held_at} times, and the chain waits`);

// A person writes, and the count starts over.
const before = (await b.chat()).messages.length;
await b.say("@ada thank you", {answered: false});
await settled(before + 2);
await b.waitFor("the count to start over", async () => !(await b.exists(".w-chain")), 100);
step("a person wrote, and the count started over");

// One is taken out, and is not in the chat.
if (!(await b.pressText(`${beside} button`, "Take out"))) cleanup(1, "nobody can be taken out");
await b.waitFor("one fewer in the chat", async () =>
  ((await b.chat())?.chat.members ?? []).filter((member) => member.kind === "agent").length === 1, 100);
const left = (await b.chat()).chat.members.filter((member) => member.kind === "agent").map((member) => member.handle);
await b.waitFor("the list to say so", async () => {
  const shown = await shownBeside();
  return left.every((handle) => shown.includes(`@${handle}`)) && ["ada", "bob"].filter((handle) => !left.includes(handle)).every((handle) => !shown.includes(`@${handle}`));
}, 100);
// And brought back by Add someone.
if (!(await b.pressText(".w-head button", "Add someone"))) cleanup(1, "nobody can be added");
await b.waitFor("who can be brought in", async () => b.exists(".w-dialog .k-rail-item"), 100);
const gone = ["ada", "bob"].find((handle) => !left.includes(handle));
if (!(await b.pressText(".w-dialog .k-rail-item", gone))) cleanup(1, `${gone} cannot be brought back`);
await b.waitFor("two in the chat again", async () =>
  ((await b.chat())?.chat.members ?? []).filter((member) => member.kind === "agent").length === 2, 100);
if (!(await b.pressText(`${beside} .k-row:has(.k-mono) button`, "Take out"))) cleanup(1, "nobody can be taken out again");
await b.waitFor("one in the chat", async () =>
  ((await b.chat())?.chat.members ?? []).filter((member) => member.kind === "agent").length === 1, 100);
step("one was taken out, brought back and taken out");

console.log("chat of several OK");
console.log(JSON.stringify({
  answered_the_one_named: answered,
  held_at,
  in_the_chat_at_the_end: (await b.chat()).chat.members.filter((member) => member.kind === "agent").map((member) => member.handle),
}));
cleanup(0, "done");
