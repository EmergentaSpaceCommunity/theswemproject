// Engine-probe browser gate driver: opens the probe App of the apps fixture
// in the REAL generic Workbench (official AppBridge, different-origin sandbox
// proxy, srcdoc view), clicks its "Run probe" button by viewport coordinates
// and waits until the App acknowledged its measurements through the relay.
// The facts themselves are read by the Rust test from the receipt file.
// usage: node workbench_apps_engine_probe_cdp_driver.mjs <browser> <url>
import {spawn} from "node:child_process";
import {mkdtempSync, readFileSync, rmSync} from "node:fs";
import {tmpdir} from "node:os";
import {join} from "node:path";

const [browser, url, appUri = "ui://apps-fixture/engine-probe"] = process.argv.slice(2);
if (!browser || !url) {
  console.error("usage: node workbench_apps_engine_probe_cdp_driver.mjs <browser> <url> [app uri]");
  process.exit(2);
}
const profile = mkdtempSync(join(tmpdir(), "swem-probe-cdp-"));
/// Chromium refuses to start its sandbox as uid 0, which is how the container
/// gates run. This is a property of the machine, not of any one driver, so it
/// is decided here once rather than threaded through ten argument lists.
const sandboxArgs = process.env.SWEM_BROWSER_NO_SANDBOX === "1" ? ["--no-sandbox"] : [];
const child = spawn(browser, [
  "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
  "--autoplay-policy=no-user-gesture-required",
  // A fake capture device answers getUserMedia without a prompt: the gate
  // measures the host's grant and the View's reach, not a person's click.
  "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream",
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
// Every frame's console, exceptions and the responses of the App's own
// documents, so a View that never speaks explains itself at the end.
const trace = [];
const describeArgs = (args) => (args || []).map((arg) => arg.value !== undefined ? String(arg.value) : (arg.description || arg.type)).join(" ");
const sendTo = (sessionId, method, params = {}) => new Promise((resolve, reject) => {
  const id = nextId++;
  pending.set(id, {resolve, reject});
  ws.send(JSON.stringify({id, sessionId, method, params}));
});
ws.onmessage = (event) => {
  const message = JSON.parse(event.data);
  if (message.method === "Target.attachedToTarget") {
    const {sessionId} = message.params;
    sendTo(sessionId, "Runtime.enable").catch(() => {});
    sendTo(sessionId, "Network.enable").catch(() => {});
  }
  if (message.method === "Runtime.consoleAPICalled" && ["error", "warning", "log"].includes(message.params.type)) {
    trace.push(`[${message.sessionId || "page"}] console.${message.params.type}: ${describeArgs(message.params.args)}`);
  }
  if (message.method === "Runtime.exceptionThrown") {
    const details = message.params.exceptionDetails || {};
    trace.push(`[${message.sessionId || "page"}] uncaught: ${(details.exception && details.exception.description) || details.text} (${details.url || ""})`);
  }
  if (message.method === "Network.responseReceived") {
    const response = message.params.response;
    if (/\/apps\/|\/sandbox/.test(response.url) && !response.url.includes("/observations/")) trace.push(`[${message.sessionId || "page"}] ${response.status} ${response.url.slice(0, 160)}`);
  }
  if (message.method === "Network.loadingFailed") {
    trace.push(`[${message.sessionId || "page"}] loading failed: ${message.params.errorText} ${message.params.blockedReason || ""}`);
  }
  if (message.id && pending.has(message.id)) {
    const {resolve, reject} = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) reject(new Error(message.error.message)); else resolve(message.result);
  }
};
const send = (method, params = {}) => sendTo(undefined, method, params);
await send("Page.enable");
await send("Runtime.enable");
await send("Network.enable");
await send("Target.setAutoAttach", {autoAttach: true, waitForDebuggerOnStart: false, flatten: true});
await send("Page.navigate", {url});

const evaluate = async (expression) => {
  const result = await send("Runtime.evaluate", {expression, returnByValue: true, awaitPromise: true});
  if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
  return result.result.value;
};
const textOf = (id) => evaluate(`(document.getElementById(${JSON.stringify(id)}) || {textContent: ""}).textContent`);
const waitFor = async (label, predicate, attempts = 150) => {
  for (let i = 0; i < attempts; i++) {
    try { if (await predicate()) return; } catch {}
    await sleep(200);
  }
  cleanup(2, `timed out waiting for ${label}; apps=${await textOf("apps-status")}\ntrace:\n  ${trace.slice(0, 80).join("\n  ")}`);
};
const click = async (selector) => {
  const rect = JSON.parse(await evaluate(
    `JSON.stringify(document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect())`));
  const x = rect.left + rect.width / 2;
  const y = rect.top + rect.height / 2;
  await send("Input.dispatchMouseEvent", {type: "mouseMoved", x, y, pointerType: "mouse"});
  await send("Input.dispatchMouseEvent", {type: "mousePressed", x, y, button: "left", buttons: 1, clickCount: 1, pointerType: "mouse"});
  await send("Input.dispatchMouseEvent", {type: "mouseReleased", x, y, button: "left", clickCount: 1, pointerType: "mouse"});
};
const clickInApp = async (x, y) => {
  const rect = JSON.parse(await evaluate(
    `JSON.stringify(document.getElementById("app-frame").getBoundingClientRect())`));
  const px = rect.left + x;
  const py = rect.top + y;
  await send("Input.dispatchMouseEvent", {type: "mouseMoved", x: px, y: py, pointerType: "mouse"});
  await send("Input.dispatchMouseEvent", {type: "mousePressed", x: px, y: py, button: "left", buttons: 1, clickCount: 1, pointerType: "mouse"});
  await send("Input.dispatchMouseEvent", {type: "mouseReleased", x: px, y: py, button: "left", clickCount: 1, pointerType: "mouse"});
};
const eventKinds = () => evaluate(
  `JSON.stringify([...document.querySelectorAll("#events li")].map((li) => li.dataset.kind))`)
  .then((value) => JSON.parse(value));

// The product lands on the Project space (the ladder is the door); the
// agent's surface is one click away, as a person reaches it.
await waitFor("the space bar", async () => evaluate(`document.getElementById("space-agent") !== null`));
await click("#space-agent");
await waitFor("the agent space", async () =>
  evaluate(`document.querySelector('.app-shell[data-active-space="agent"]') !== null`));
await waitFor("profile", async () =>
  (await evaluate(`document.getElementById("profiles").options.length`)) >= 1);
await click("#open-new");
// Sixty seconds, not thirty. Opening an agent connection starts a process
// and waits for its handshake, and under a full gate - several browser
// suites, each with its own Chromium and its own agent, running at once -
// thirty is not always enough: this wait is what the apps probe timed out
// on in an otherwise green gate. Three of the six walks that wait on this
// same step already carried 300; the other three carried the default for
// no reason but that nobody had hit it yet.
await waitFor("connection open", async () => (await textOf("connection-id")) !== "-", 300);
const route = await textOf("route");
const probeSelector = `#apps-list .app-row[data-uri="${appUri}"] .app-open`;
await waitFor("probe App discovered", async () =>
  (await evaluate(`document.querySelectorAll(${JSON.stringify(probeSelector)}).length`)) === 1);
await click(probeSelector);
await waitFor("App ready", async () => (await textOf("apps-status")) === "app ready", 300);
await sleep(500);
// The probe button sits at (20,60) 200x36 inside the App.
await clickInApp(120, 78);
await waitFor("probe acknowledged through the relay", async () =>
  (await eventKinds()).filter((kind) => kind === "host/app_tool_call").length >= 1, 300);
await sleep(300);
// The facts are persisted by now; closing the App and the connection is
// ordinary teardown the Rust test also performs, so it is best-effort here.
try {
  await click("#app-close");
  await sleep(500);
  await click("#disconnect");
  await sleep(500);
} catch {}
console.log(JSON.stringify({route}));
cleanup(0);
