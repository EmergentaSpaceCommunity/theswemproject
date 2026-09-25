// A person's sessions: start one, start another, come back to the first.
// usage: node <this-file> <browser> <workbench-url> [--reopen]
//
// The second session is the whole point. Until this gate, pressing "Start
// session" a second time failed forever with "route binding drift", because
// the page handed the host the route the first session had bound.
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [browser, url, mode, wanted] = process.argv.slice(2);
if (!browser || !url) {
  console.error("usage: node workbench_shell_sessions_cdp_driver.mjs <browser> <url> [--reopen]");
  process.exit(2);
}
const reopening = mode === "--reopen";
const profile = mkdtempSync(join(tmpdir(), "swem-sessions-cdp-"));
const sandboxArgs = process.env.SWEM_BROWSER_NO_SANDBOX === "1" ? ["--no-sandbox"] : [];
const child = spawn(browser, [
  "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
  "--window-size=1280,900", `--user-data-dir=${profile}`, "--remote-debugging-port=0",
  ...sandboxArgs, "about:blank",
], {stdio: "ignore"});
// A verdict ends the walk where it was reached. `cleanup` used to arm
// `process.exit` on a timer and return, so the driver carried on and printed
// steps it never took, and the exit code that decided the walk survived only
// because one timer was longer than another. The latch makes the
// first verdict final; the throw stops the caller.
const walkStopped = {walkStopped: true};
let walkHasItsVerdict = false;
const cleanup = (code, message) => {
  if (walkHasItsVerdict) { throw walkStopped; }
  if (message) console.error(message);
  walkHasItsVerdict = true;
  try { child.kill(); } catch {}
  setTimeout(() => { try { rmSync(profile, {recursive: true, force: true}); } catch {} ; process.exit(code); }, 300);
  throw walkStopped;
};
process.on("uncaughtException", (error) => {
  if (error && error.walkStopped === true) { return; }
  try { cleanup(2, `driver error: ${error.stack || error}`); } catch {}
});
process.on("unhandledRejection", (error) => {
  if (error && error.walkStopped === true) { return; }
  try { cleanup(2, `driver rejection: ${error.stack || error}`); } catch {}
});
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

let port = null;
for (let i = 0; i < 100 && port === null; i++) {
  await sleep(200);
  try { port = Number(readFileSync(join(profile, "DevToolsActivePort"), "utf8").split("\n")[0]); }
  catch {}
}
if (!port) cleanup(2, "browser never opened a DevTools port");
let target = null;
for (let i = 0; i < 50 && !target; i++) {
  await sleep(200);
  try {
    const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    target = list.find((entry) => entry.type === "page");
  } catch {}
}
if (!target) cleanup(2, "no page target");

const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
let nextId = 1;
const pending = new Map();
ws.onmessage = (event) => {
  const message = JSON.parse(event.data);
  if (message.id && pending.has(message.id)) {
    const {resolve, reject} = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) reject(new Error(message.error.message)); else resolve(message.result);
  }
};
const send = (method, params = {}) => new Promise((resolve, reject) => {
  const id = nextId++;
  pending.set(id, {resolve, reject});
  ws.send(JSON.stringify({id, method, params}));
});
await send("Page.enable");
await send("Runtime.enable");
await send("Page.navigate", {url});

const evaluate = async (expression) => {
  const result = await send("Runtime.evaluate", {expression, returnByValue: true, awaitPromise: true});
  if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
  return result.result.value;
};
const textOf = (id) => evaluate(`(document.getElementById(${JSON.stringify(id)}) || {textContent: ""}).textContent`);
const state = async () => {
  const said = JSON.stringify(await saidHere().catch(() => []));
  const history = await evaluate(`fetch("/api/routes/" + encodeURIComponent(
    (document.getElementById("route")||{textContent:""}).textContent.trim()) + "/history?limit=50")
    .then((response) => response.text()).then((text) => text.slice(0, 300)).catch((error) => String(error))`)
    .catch((error) => String(error));
  return `phase=${await textOf("phase")} problem=${await textOf("close-error")} route=${await textOf("route")} said=${said} history=${history}`;
};
const waitFor = async (what, predicate, attempts = 150) => {
  for (let index = 0; index < attempts; index += 1) {
    try { if (await predicate()) return; } catch {}
    await sleep(200);
  }
  cleanup(2, `timed out waiting for ${what}; ${await state()}`);
};
const click = async (selector) => {
  // Bring it into view first: a control below the fold of a scrolling rail
  // has a viewport point outside the window, and a click there lands nowhere.
  await evaluate(`document.querySelector(${JSON.stringify(selector)})?.scrollIntoView({block: "center", inline: "center", behavior: "instant"})`);
  const rect = JSON.parse(await evaluate(
    `JSON.stringify(document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect())`));
  const x = rect.left + rect.width / 2;
  const y = rect.top + rect.height / 2;
  await send("Input.dispatchMouseEvent", {type: "mouseMoved", x, y, pointerType: "mouse"});
  await send("Input.dispatchMouseEvent", {type: "mousePressed", x, y, button: "left", buttons: 1, clickCount: 1, pointerType: "mouse"});
  await send("Input.dispatchMouseEvent", {type: "mouseReleased", x, y, button: "left", clickCount: 1, pointerType: "mouse"});
};
const type = async (selector, text) => {
  await click(selector);
  await send("Input.insertText", {text});
};
const sessionRows = () => evaluate(
  `JSON.stringify([...document.querySelectorAll("#sessions .session-row")].map((row) => row.dataset.routeId))`)
  .then(JSON.parse);
const saidHere = () => evaluate(
  `JSON.stringify([...document.querySelectorAll("#conversation .message .message-body")].map((node) => node.textContent))`)
  .then(JSON.parse);

await waitFor("the workbench", async () => evaluate(`window.__SWEM_WORKBENCH_WEB__ === "react-19"`));
// The product lands on the Project space (the ladder is the door); the
// agent's surface is one click away, as a person reaches it.
await waitFor("the space bar", async () => evaluate(`document.getElementById("space-agent") !== null`));
await click("#space-agent");
await waitFor("the agent space", async () =>
  evaluate(`document.querySelector('.app-shell[data-active-space="agent"]') !== null`));
await waitFor("the profile", async () => (await evaluate(`document.getElementById("profiles").options.length`)) >= 1);

// On the second run of the product, the sessions of the first run must be
// listed before anything else happens, and still hold what was said.
if (reopening) {
  await waitFor("both sessions listed after a restart", async () => (await sessionRows()).length === 2);
  const rows = await sessionRows();
  if (!wanted || !rows.includes(wanted)) cleanup(2, `the session to come back to is not listed: ${JSON.stringify(rows)}`);
  await click(`#sessions .session-row[data-route-id="${wanted}"]`);
  await waitFor("that session opens", async () =>
    (await textOf("route")) === wanted && (await textOf("connection-id")) !== "-", 300);
  await waitFor("with what was said in it", async () =>
    (await saidHere()).some((text) => text.includes("the first thing")));
  await click("#disconnect");
  await waitFor("disconnected", async () => evaluate(`document.getElementById("disconnect").disabled`));
  console.log(JSON.stringify({sessions: rows, reopened: true}));
  await sleep(200);
  cleanup(0);
}

// 1. A first session, with something said in it.
await waitFor("no sessions yet", async () => evaluate(`document.getElementById("sessions-empty") !== null`));
await click("#open-new");
await waitFor("the first session opens", async () => (await textOf("connection-id")) !== "-", 300);
const first = await textOf("route");
await type("#prompt-text", "the first thing");
await click("#send");
await waitFor("the first turn ends", async () => (await textOf("turn-outcome")).startsWith("turn:"), 300);
await click("#disconnect");
await waitFor("the first session ends", async () => evaluate(`document.getElementById("disconnect").disabled`));

// 2. A second session. This is what used to fail forever.
await click("#open-new");
await waitFor("a second session opens", async () => {
  const problem = await textOf("close-error");
  if (problem.includes("drift")) cleanup(2, `the second session still drifts: ${problem}`);
  return (await textOf("connection-id")) !== "-" && (await textOf("route")) !== first;
}, 300);
const second = await textOf("route");
await type("#prompt-text", "the second thing");
await click("#send");
await waitFor("the second turn ends", async () => (await textOf("turn-outcome")).startsWith("turn:"), 300);

// 3. Both are listed, and the first can be picked up again with its own words.
await waitFor("both sessions listed", async () => (await sessionRows()).length === 2);
await click(`#sessions .session-row[data-route-id="${first}"]`);
await waitFor("the first session is open again", async () =>
  (await textOf("route")) === first && (await textOf("connection-id")) !== "-", 300);
await waitFor("its conversation came back", async () => {
  const said = await saidHere();
  return said.some((text) => text.includes("the first thing")) && !said.some((text) => text.includes("the second thing"));
});
const recovered = await saidHere();
await click("#disconnect");
await waitFor("disconnected", async () => evaluate(`document.getElementById("disconnect").disabled`));

console.log(JSON.stringify({first, second, recovered}));
await sleep(200);
cleanup(0);
