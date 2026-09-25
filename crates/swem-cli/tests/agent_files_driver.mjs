// A person hands their agent a file, and the agent hands one back.
//
// Neither direction uses a protocol for files. The attachment lands in a
// directory inside the agent's own workspace and the turn tells the agent
// where; the agent opens that path, and writes its answer to another path in
// the same workspace. The page shows both without being told.

import {mkdtempSync, rmSync, writeFileSync} from "node:fs";
import {tmpdir} from "node:os";
import {join} from "node:path";

import {launchBrowser, sleep, cleanup} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, handed, secret, answer] = process.argv.slice(2);
const browser = process.env.SWEM_BROWSER || "/opt/pw-browsers/chromium";
if (!url || !handed || !secret || !answer) {
  console.error("usage: agent_files_driver.mjs <url> <file> <secret> <answer>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "agent-files"});

const setValue = async (id, value, kind = "HTMLInputElement") =>
  b.evaluate(`(() => {
    const field = document.getElementById(${JSON.stringify(id)});
    Object.getOwnPropertyDescriptor(window.${kind}.prototype, 'value').set.call(field, ${JSON.stringify(value)});
    field.dispatchEvent(new Event('input', {bubbles: true}));
  })()`);

const said = () => b.evaluate(
  `JSON.stringify([...document.querySelectorAll("#conversation .message .message-body")].map(node => node.textContent))`,
).then(JSON.parse);

const say = async (text) => {
  // `#turn-outcome` still reads the PREVIOUS turn's `turn: …` at the moment
  // the next one is sent: the page resets it when the prompt goes out, and
  // waiting only for `turn:` is already satisfied on the way in. Under the
  // load of a full gate run this walk read the transcript before the answer
  // it was waiting for existed, and reported the escape attempt unrefused
  // while the escape had not been tried yet. So the wait is for a turn that
  // is both over and this one: the person's own message and the agent's
  // answer, both new, under an outcome that has come back.
  const before = (await said()).length;
  await setValue("prompt-text", text, "HTMLTextAreaElement");
  await b.click("#send");
  await b.waitFor("the turn ends", async () =>
    b.evaluate(`(() => {
      const outcome = (document.getElementById("turn-outcome")||{textContent:""}).textContent;
      const messages = document.querySelectorAll("#conversation .message .message-body").length;
      return outcome.startsWith("turn:") && messages >= ${before} + 2;
    })()`), 300);
  return said();
};

step("the product opens");
await b.waitFor("the shell", async () => b.exists("#space-agent"));
await b.click("#space-agent");

// The person's own agent, declared on this machine, used with one click.
await b.waitFor("the agent offered", async () => b.exists("#agent-options .agent-option"));
await b.click('[data-agent-id="hands"]');
await b.waitFor("a profile exists", async () =>
  b.evaluate(`document.querySelectorAll("#profiles option").length > 0`), 200);
step("the agent is theirs");

await b.click("#open-new");
await b.waitFor("the session opens", async () =>
  b.evaluate(`(document.getElementById("route")||{textContent:"-"}).textContent !== "-"`), 300);
step("a session is open");

// Nothing has been handed over yet, and the page says so rather than showing
// a list that is wrong.
await b.click('[data-agent-tab="files"]');
await b.waitFor("the Files panel", async () => b.exists('[data-agent-panel="files"]:not([hidden])'));
if (!(await b.exists('[data-files-empty="inbox"]'))) cleanup(1, "the inbox is not empty before anything was handed over");
if (!(await b.exists('[data-files-empty="outbox"]'))) cleanup(1, "the outbox is not empty before the agent wrote anything");
step("both directories start empty");

// Hand the file over: the same Attach a person uses for anything else.
await b.click('[data-agent-tab="conversation"]');
if (!(await b.setFileInputFiles("#file-input", [handed]))) cleanup(2, "the composer has no file input");
await b.waitFor("the attachment is listed", async () =>
  b.evaluate(`(document.getElementById("attachment-list")||{textContent:""}).textContent.length > 0`));
const name = handed.split("/").pop();
// The agent is told to read it by the path the product gave it, which is what
// a person would do: the turn carries that path as a resource link.
const read = await say(JSON.stringify({read: `inbox/${name}`}));
if (!read.some((text) => text.includes(secret))) {
  cleanup(1, `the agent did not read what was handed to it: ${JSON.stringify(read)}`);
}
step("the agent opened the file by path");

// And the page shows it in the inbox, without having been told.
await b.click('[data-agent-tab="files"]');
await b.waitFor("the handed file is listed", async () =>
  b.exists(`[data-files-area="inbox"] [data-file-name=${JSON.stringify(name)}]`), 200);
step("the inbox lists it");

// The other direction: the agent writes a file where it works, and the person
// finds it. No resource link, no tool, no capability.
await b.click('[data-agent-tab="conversation"]');
const wrote = await say(JSON.stringify({write: "outbox/reply.txt", text: answer}));
if (!wrote.some((text) => text.includes("wrote"))) {
  cleanup(1, `the agent did not write anything: ${JSON.stringify(wrote)}`);
}
await b.click('[data-agent-tab="files"]');
await b.waitFor("the answer is listed", async () =>
  b.exists('[data-files-area="outbox"] [data-file-name="reply.txt"]'), 200);
step("the outbox lists what the agent wrote");

// Opening it is the point: the person reads the bytes, through the same door
// the rest of the page uses.
const href = await b.evaluate(
  `document.querySelector('[data-files-area="outbox"] [data-file-name="reply.txt"] a')?.getAttribute("href")`);
const served = await b.evaluate(`(async () => {
  const response = await fetch(${JSON.stringify(href)});
  return JSON.stringify({status: response.status, type: response.headers.get("content-type"), body: await response.text()});
})()`).then(JSON.parse);
if (served.status !== 200 || served.body !== answer) {
  cleanup(1, `the outbox did not serve what the agent wrote: ${JSON.stringify(served)}`);
}
step(`the person read it back (${served.type})`);

// The agent works inside its own directory and nowhere else: the same write,
// aimed out of the workspace, is refused rather than done.
await b.click('[data-agent-tab="conversation"]');
const escaped = await say(JSON.stringify({write: "../escaped.txt", text: "should not exist"}));
if (!escaped.some((text) => /outside/.test(text))) {
  cleanup(1, `writing outside the workspace was not refused: ${JSON.stringify(escaped)}`);
}
step("a write out of the workspace is refused");

// Two files handed over in one turn. The control has always taken several -
// it carries `multiple`, and the prompt body takes a list of them - and every
// walk had handed it exactly one, so the loop that writes them into the
// agent's workspace had never run twice for one turn.
const bothDir = mkdtempSync(join(tmpdir(), "swem-two-attachments-"));
const both = [
  {name: "first-handed.txt", secret: `${secret}-one`},
  {name: "second-handed.txt", secret: `${secret}-two`},
].map((one) => {
  const path = join(bothDir, one.name);
  writeFileSync(path, `${one.secret}\n`);
  return {...one, path};
});
if (!(await b.setFileInputFiles("#file-input", both.map((one) => one.path)))) {
  cleanup(2, "the composer has no file input");
}
await b.waitFor("both attachments listed", async () => {
  const listed = await b.evaluate(`(document.getElementById("attachment-list")||{textContent:""}).textContent`);
  return both.every((one) => listed.includes(one.name));
});
// The agent reads the second one, by the path the product gave it. Reading
// the one that came last is the question: a turn that carried only the first
// would answer for the first.
const readSecond = await say(JSON.stringify({read: `inbox/${both[1].name}`}));
if (!readSecond.some((text) => text.includes(both[1].secret))) {
  cleanup(1, `the second of two files handed over in one turn did not reach the agent: ${JSON.stringify(readSecond)}`);
}
await b.click('[data-agent-tab="files"]');
for (const one of both) {
  await b.waitFor(`the inbox lists ${one.name}`, async () =>
    b.exists(`[data-files-area="inbox"] [data-file-name=${JSON.stringify(one.name)}]`), 200);
}
rmSync(bothDir, {recursive: true, force: true});
step("two files in one turn reach the agent and the inbox");

await sleep(200);
console.log("files OK");
console.log(JSON.stringify({href, served}));
cleanup(0, "agent files done");
