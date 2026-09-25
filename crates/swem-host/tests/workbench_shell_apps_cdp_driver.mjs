// Dependency-free Chrome DevTools Protocol driver for the generic Apps host
// gate (H1). usage: node workbench_shell_apps_cdp_driver.mjs <browser> <url>
//
// Drives the REAL shell page: opens a connection on the notes profile, opens
// the discovered App (official AppBridge -> different-origin sandbox proxy ->
// srcdoc view), clicks and types INSIDE the sandboxed App by viewport
// coordinates (OOPIF - never scripted), saves a note whose receipt is the
// oracle, disconnects and resumes to prove resource identity, runs the
// hostile App and watches the host refuse every attempt, and proves the
// shell survives. DOM reads happen only on the shell page.
import {spawn} from "node:child_process";
import {mkdtempSync, readFileSync, rmSync} from "node:fs";
import {tmpdir} from "node:os";
import {join} from "node:path";

const [browser, url] = process.argv.slice(2);
if (!browser || !url) {
  console.error("usage: node workbench_shell_apps_cdp_driver.mjs <browser> <url>");
  process.exit(2);
}
const profile = mkdtempSync(join(tmpdir(), "swem-apps-cdp-"));
/// Chromium refuses to start its sandbox as uid 0, which is how the container
/// gates run. This is a property of the machine, not of any one driver, so it
/// is decided here once rather than threaded through ten argument lists.
const sandboxArgs = process.env.SWEM_BROWSER_NO_SANDBOX === "1" ? ["--no-sandbox"] : [];
const child = spawn(browser, [
  "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
  "--window-size=1280,1000", `--user-data-dir=${profile}`, "--remote-debugging-port=0",
  ...sandboxArgs, "about:blank"
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
const waitFor = async (label, predicate, attempts = 150, interval = 200) => {
  for (let i = 0; i < attempts; i++) {
    try { if (await predicate()) return; } catch {}
    await sleep(interval);
  }
  let snapshot = "";
  try {
    snapshot = ` (apps-status=${await textOf("apps-status")}; terminal=${await textOf("terminal")}; phase=${await textOf("phase")})`;
  } catch {}
  cleanup(2, `timed out waiting for ${label}${snapshot}`);
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
// A click INSIDE the sandboxed App: viewport point = app-frame rect + the
// fixture's fixed geometry (OOPIF - coordinates, never scripting).
const appFrameRect = async () => JSON.parse(await evaluate(
  `JSON.stringify(document.getElementById("app-frame").getBoundingClientRect())`));
const clickInApp = async (x, y) => {
  const rect = await appFrameRect();
  const px = rect.left + x;
  const py = rect.top + y;
  await send("Input.dispatchMouseEvent", {type: "mouseMoved", x: px, y: py, pointerType: "mouse"});
  await send("Input.dispatchMouseEvent", {type: "mousePressed", x: px, y: py, button: "left", buttons: 1, clickCount: 1, pointerType: "mouse"});
  await send("Input.dispatchMouseEvent", {type: "mouseReleased", x: px, y: py, button: "left", clickCount: 1, pointerType: "mouse"});
};
const typePrompt = async (text) => {
  await evaluate(`document.getElementById("prompt-text").value = ""`);
  await click("#prompt-text");
  await send("Input.insertText", {text});
};
const eventKinds = () => evaluate(
  `JSON.stringify([...document.querySelectorAll("#events li")].map((li) => li.dataset.kind))`)
  .then((value) => JSON.parse(value));

// --- Stage 1: notes profile, open the discovered App ----------------------
// The product lands on the Project space (the ladder is the door); the
// agent's surface is one click away, as a person reaches it.
await waitFor("the space bar", async () => evaluate(`document.getElementById("space-agent") !== null`));
await click("#space-agent");
await waitFor("the agent space", async () =>
  evaluate(`document.querySelector('.app-shell[data-active-space="agent"]') !== null`));
await waitFor("profiles loaded", async () =>
  (await evaluate(`document.getElementById("profiles").options.length`)) >= 2);
await evaluate(`document.getElementById("profiles").value = "apps-main"`);
await click("#open-new");
// Sixty seconds for the handshake, as every other walk that waits on this
// step: thirty is not enough under a full gate.
await waitFor("connection open", async () => (await textOf("connection-id")) !== "-", 300);
const mainRoute = await textOf("route");
await waitFor("App discovered", async () =>
  (await evaluate(`document.querySelectorAll("#apps-list .app-row .app-open").length`)) > 0);
const listedUri = await evaluate(
  `document.querySelector("#apps-list .app-row").dataset.uri`);
await click("#apps-list .app-row .app-open");
await waitFor("App ready", async () => (await textOf("apps-status")) === "app ready", 300);
const appUri = await textOf("app-uri");
const appServer = await textOf("app-server");
const appPermissions = await textOf("app-permissions");
if (appUri !== listedUri) cleanup(2, `opened uri ${appUri} differs from listed ${listedUri}`);

// Real input INSIDE the sandboxed App (fixture geometry: nonce at 20,60
// 300x30; text at 20,110; save button at 20,160 160x36).
await sleep(500);
await clickInApp(170, 75);
await send("Input.insertText", {text: "browser-app-nonce"});
await clickInApp(170, 125);
await send("Input.insertText", {text: "clicked inside a real sandboxed App"});
await clickInApp(100, 178);
await waitFor("allowed tool call on the lane", async () =>
  (await eventKinds()).includes("host/app_tool_call"));

// --- Stage 2: resume preserves the resource identity ----------------------
await click("#app-close");
await waitFor("app closed", async () => (await eventKinds()).includes("host/app_closed"));
await click("#disconnect");
await waitFor("disconnected", async () => evaluate(`document.getElementById("disconnect").disabled`));
await click("#open-resume");
await waitFor("resumed connection", async () =>
  (await textOf("route")) === mainRoute && (await textOf("connection-id")) !== "-");
await waitFor("App re-discovered", async () =>
  (await evaluate(`document.querySelectorAll("#apps-list .app-row .app-open").length`)) > 0);
await click("#apps-list .app-row .app-open");
await waitFor("App ready again", async () => (await textOf("apps-status")) === "app ready", 300);
const appUriAfterResume = await textOf("app-uri");
const appServerAfterResume = await textOf("app-server");
if (appUriAfterResume !== appUri || appServerAfterResume !== appServer) {
  cleanup(2, `resource identity changed across resume: ${appServerAfterResume} ${appUriAfterResume}`);
}
// The resumed session shows its lane from the start, so the first close is
// already listed: wait for a second one rather than for the kind to exist.
const closesBefore = (await eventKinds()).filter((kind) => kind === "host/app_closed").length;
await click("#app-close");
await waitFor("second app closed", async () =>
  (await eventKinds()).filter((kind) => kind === "host/app_closed").length > closesBefore);
await click("#disconnect");
await waitFor("resumed disconnected", async () =>
  evaluate(`document.getElementById("disconnect").disabled`));

// --- Stage 3: the hostile App is refused and the shell survives -----------
await evaluate(`document.getElementById("profiles").value = "apps-hostile"`);
await evaluate(`document.getElementById("route-id").value = ""`);
await click("#open-new");
await waitFor("hostile connection open", async () =>
  (await textOf("route")) !== mainRoute && (await textOf("connection-id")) !== "-");
const hostileRoute = await textOf("route");
await waitFor("hostile App discovered", async () =>
  (await evaluate(`document.querySelectorAll("#apps-list .app-row .app-open").length`)) > 0);
await click("#apps-list .app-row .app-open");
// The hostile App fires its attempts right after the handshake; the shell's
// refusal counter is fed by the durable host/app_tool_call events. The
// model-only, undeclared and cross-server tool calls reach the Rust relay
// and are refused there; the ui/* attempt is intercepted one layer earlier
// by the official bridge itself (the Rust ui/* gate is component-proven).
await waitFor("refusals recorded", async () =>
  Number(await textOf("app-refusals")) >= 3, 300);
await typePrompt("still alive after the hostile app");
await click("#send");
await waitFor("post-hostile turn", async () =>
  (await textOf("turn-outcome")).startsWith("turn:"));
await click("#disconnect");
await waitFor("hostile disconnected", async () =>
  evaluate(`document.getElementById("disconnect").disabled`));

console.log(JSON.stringify({
  main_route: mainRoute,
  hostile_route: hostileRoute,
  app_server: appServer,
  app_uri: appUri,
  app_permissions: appPermissions,
}));
cleanup(0);
