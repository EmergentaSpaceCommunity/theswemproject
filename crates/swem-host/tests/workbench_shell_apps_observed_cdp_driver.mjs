// Real-browser driver for the agent-initiated MCP Apps lifecycle.
// It never scripts the sandboxed App: the acknowledgement is a viewport
// click, while external receipt files remain the semantic oracle.
import {spawn} from "node:child_process";
import {mkdtempSync, readFileSync, rmSync, writeFileSync} from "node:fs";
import {tmpdir} from "node:os";
import {join} from "node:path";

const [browser, url] = process.argv.slice(2);
if (!browser || !url) process.exit(2);
const profile = mkdtempSync(join(tmpdir(), "swem-observed-app-cdp-"));
/// Chromium refuses to start its sandbox as uid 0, which is how the container
/// gates run. This is a property of the machine, not of any one driver, so it
/// is decided here once rather than threaded through ten argument lists.
const sandboxArgs = process.env.SWEM_BROWSER_NO_SANDBOX === "1" ? ["--no-sandbox"] : [];
const child = spawn(browser, [
  "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
  "--window-size=1280,1000", `--user-data-dir=${profile}`, "--remote-debugging-port=0",
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
  setTimeout(() => {
    try { rmSync(profile, {recursive: true, force: true}); } catch {}
    process.exit(code);
  }, 300);
  throw walkStopped;
};
process.on("uncaughtException", (error) => {
  if (error && error.walkStopped === true) { return; }
  try { cleanup(2, String(error.stack || error)); } catch {}
});
process.on("unhandledRejection", (error) => {
  if (error && error.walkStopped === true) { return; }
  try { cleanup(2, String(error.stack || error)); } catch {}
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
    const request = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) request.reject(new Error(message.error.message));
    else request.resolve(message.result);
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
const textOf = (id) => evaluate(
  `(document.getElementById(${JSON.stringify(id)}) || {textContent: ""}).textContent`);
const waitFor = async (label, predicate, attempts = 300) => {
  for (let i = 0; i < attempts; i++) {
    try { if (await predicate()) return; } catch {}
    await sleep(200);
  }
  let evidence = "";
  try {
    const shot = await send("Page.captureScreenshot", {format: "png"});
    const path = join(process.env.SWEM_FAILURE_DIR || tmpdir(), "gate-failure-observed.png");
    writeFileSync(path, Buffer.from(shot.data, "base64"));
    const frame = await evaluate(`JSON.stringify({frame: document.getElementById("app-frame")?.getBoundingClientRect(), inner: {w: innerWidth, h: innerHeight}})`);
    evidence = ` screenshot=${path} geometry=${frame}`;
  } catch {}
  cleanup(2, `timed out waiting for ${label}; status=${await textOf("apps-status")};${evidence}`);
};
const click = async (selector) => {
  // Bring it into view first: a control below the fold of a scrolling rail
  // has a viewport point outside the window, and a click there lands nowhere.
  await evaluate(`document.querySelector(${JSON.stringify(selector)})?.scrollIntoView({block: "center", inline: "center", behavior: "instant"})`);
  const rect = JSON.parse(await evaluate(
    `JSON.stringify(document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect())`));
  const x = rect.left + rect.width / 2;
  const y = rect.top + rect.height / 2;
  await send("Input.dispatchMouseEvent", {type: "mousePressed", x, y, button: "left", buttons: 1, clickCount: 1});
  await send("Input.dispatchMouseEvent", {type: "mouseReleased", x, y, button: "left", clickCount: 1});
};
const clickInApp = async (x, y) => {
  const rect = JSON.parse(await evaluate(
    `JSON.stringify(document.getElementById("app-frame").getBoundingClientRect())`));
  const px = rect.left + x;
  const py = rect.top + y;
  await send("Input.dispatchMouseEvent", {type: "mousePressed", x: px, y: py, button: "left", buttons: 1, clickCount: 1});
  await send("Input.dispatchMouseEvent", {type: "mouseReleased", x: px, y: py, button: "left", clickCount: 1});
};
const eventKinds = () => evaluate(
  `JSON.stringify([...document.querySelectorAll("#events li")].map((li) => li.dataset.kind))`)
  .then(JSON.parse);

// The product lands on the Project space (the ladder is the door); the
// agent's surface is one click away, as a person reaches it.
await waitFor("the space bar", async () => evaluate(`document.getElementById("space-agent") !== null`));
await click("#space-agent");
await waitFor("the agent space", async () =>
  evaluate(`document.querySelector('.app-shell[data-active-space="agent"]') !== null`));
await waitFor("profile", async () =>
  (await evaluate(`document.getElementById("profiles").options.length`)) === 1);
await click("#open-new");
await waitFor("connection", async () => (await textOf("connection-id")) !== "-");
const route = await textOf("route");
const prompt = JSON.stringify({
  fixture: "mcp-apps-note-v0.1",
  server: "notes",
  nonce: "browser-agent-nonce-0.55",
  text: "exact native call projected through AppBridge",
});
await evaluate(`document.getElementById("prompt-text").value = ${JSON.stringify(prompt)}`);
await click("#send");
// A whole agent turn (echo agent -> observed stdio -> notes fixture -> the
// host's projection -> AppBridge delivery) takes seconds on a quiet box and
// well over 30 s on a loaded one; wait as long as the prompt gates do.
await waitFor("agent App result", async () =>
  (await textOf("apps-status")) === "agent App result delivered", 600);
if ((await textOf("app-uri")) !== "ui://apps-fixture/notes") {
  cleanup(2, "the observed call opened another App resource");
}
await sleep(500);
// Fixture geometry: the App-only acknowledgement button is at 20,210. The
// App enables it when the result lands in its frame, a beat after the host
// says so; a click that came too early is simply made again.
const acknowledged = async () =>
  (await eventKinds()).filter((kind) => kind === "host/app_tool_call").length >= 1;
for (let attempt = 0; attempt < 8 && !(await acknowledged()); attempt += 1) {
  await clickInApp(130, 228);
  for (let tick = 0; tick < 10 && !(await acknowledged()); tick += 1) await sleep(200);
}
await waitFor("App-only acknowledgement call", acknowledged);
await click("#app-close");
await click("#disconnect");
// Disconnect awaits the runner, the projector and the observed child's real
// process boundary; give it the same minute the other gates allow.
await waitFor("disconnect", async () => evaluate(`document.getElementById("disconnect").disabled`), 300);

console.log(JSON.stringify({route, app_server: await textOf("app-server")}));
cleanup(0);
