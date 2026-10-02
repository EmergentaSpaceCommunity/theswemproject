// A person hands their agent a file, and the agent hands one back.
//
// Neither direction uses a protocol for files. The attachment lands in a
// directory inside the agent's own workspace and the turn tells the agent
// where; the agent opens that path, and writes its answer to another path in
// the same workspace. The page shows both without being told.

import {mkdtempSync, rmSync, writeFileSync} from "node:fs";
import {tmpdir} from "node:os";
import {join} from "node:path";

import {launchBrowser, sleep, cleanup, browserPath} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, handed, secret, answer] = process.argv.slice(2);
const browser = browserPath();
if (!url || !handed || !secret || !answer) {
  console.error("usage: agent_files_driver.mjs <url> <file> <secret> <answer>");
  process.exit(2);
}

const b = await launchBrowser({browser, url, label: "agent-files"});

step("the product opens");

// The person's own agent, declared on this machine, made from the page.
const profile = await b.makeAgent("hands");
step("the agent is theirs");

// Nothing has been handed over yet, and the page says so rather than showing
// a list that is wrong.
await b.openAgent(profile, "files");
await b.waitFor("the Files panel", async () => b.exists('[data-agent-panel="files"]:not([hidden])'));
if (!(await b.exists('[data-files-empty="inbox"]'))) cleanup(1, "the inbox is not empty before anything was handed over");
if (!(await b.exists('[data-files-empty="outbox"]'))) cleanup(1, "the outbox is not empty before the agent wrote anything");
step("both directories start empty");

// Hand the file over: the same Attach a person uses for anything else.
await b.openAgent(profile);
const name = handed.split("/").pop();
// The agent is told to read it by the path the product gave it, which is what
// a person would do: the turn carries that path as a resource link.
const read = await b.say(JSON.stringify({read: `inbox/${name}`}), {files: [handed]});
if (!read.some((text) => text.includes(secret))) {
  cleanup(1, `the agent did not read what was handed to it: ${JSON.stringify(read)}`);
}
step("the agent opened the file by path");

// And the page shows it in the inbox, without having been told.
await b.openAgent(profile, "files");
await b.waitFor("the handed file is listed", async () =>
  b.exists(`[data-files-area="inbox"] [data-file-name=${JSON.stringify(name)}]`), 200);
step("the inbox lists it");

// The other direction: the agent writes a file where it works, and the person
// finds it. No resource link, no tool, no capability.
await b.openAgent(profile);
const wrote = await b.say(JSON.stringify({write: "outbox/reply.txt", text: answer}));
if (!wrote.some((text) => text.includes("wrote"))) {
  cleanup(1, `the agent did not write anything: ${JSON.stringify(wrote)}`);
}
await b.openAgent(profile, "files");
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
await b.openAgent(profile);
const escaped = await b.say(JSON.stringify({write: "../escaped.txt", text: "should not exist"}));
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
// The agent reads the second one, by the path the product gave it. Reading
// the one that came last is the question: a turn that carried only the first
// would answer for the first.
const readSecond = await b.say(JSON.stringify({read: `inbox/${both[1].name}`}), {files: both.map((one) => one.path)});
if (!readSecond.some((text) => text.includes(both[1].secret))) {
  cleanup(1, `the second of two files handed over in one turn did not reach the agent: ${JSON.stringify(readSecond)}`);
}
await b.openAgent(profile, "files");
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
