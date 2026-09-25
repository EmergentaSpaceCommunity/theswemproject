// Real-browser gate for agent-initiated MCP Apps terminal states. The driver
// controls only the outer Workbench viewport; App receipts and the routing
// ledger remain the independent semantic oracles.
import {spawn} from "node:child_process";
import {mkdtempSync, readFileSync, rmSync, writeFileSync} from "node:fs";
import {tmpdir} from "node:os";
import {join} from "node:path";

const [browser, url, delayRelease] = process.argv.slice(2);
if (!browser || !url || !delayRelease) process.exit(2);
const profile = mkdtempSync(join(tmpdir(), "swem-terminal-app-cdp-"));
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
  cleanup(2, `timed out waiting for ${label}; apps=${await textOf("apps-status")}; turn=${await textOf("turn-outcome")}`);
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

async function openAndPrompt(prompt) {
  // A fresh fixture connection must not accidentally reuse the previous
  // durable route left in the shell's handoff field.
  await evaluate(`document.getElementById("route-id").value = ""`);
  await click("#open-new");
  await waitFor("connection", async () => (await textOf("connection-id")) !== "-");
  const route = await textOf("route");
  await evaluate(`document.getElementById("prompt-text").value = ${JSON.stringify(JSON.stringify(prompt))}`);
  await click("#send");
  return route;
}

async function closeAndDisconnect() {
  await click("#app-close");
  await waitFor("App close", async () => (await textOf("apps-status")) === "app closed");
  await click("#disconnect");
  await waitFor("disconnect", async () => evaluate(`document.getElementById("disconnect").disabled`));
}

const errorRoute = await openAndPrompt({
  fixture: "mcp-apps-error-v0.1", server: "notes",
  nonce: "browser-tool-error-0.57", text: "deliver exact isError result",
});
await waitFor("tool error delivery", async () =>
  (await textOf("apps-status")) === "agent App tool error delivered");
await waitFor("tool error App acknowledgement", async () =>
  (await eventKinds()).includes("host/app_tool_call"));
await waitFor("tool error native turn", async () =>
  (await textOf("turn-outcome")).includes("turn: end_turn / completed"));
await closeAndDisconnect();

const cancelRoute = await openAndPrompt({
  fixture: "mcp-apps-cancel-v0.1", server: "notes",
  nonce: "browser-cancel-0.57", text: "native MCP cancellation",
});
await waitFor("cancellation delivery", async () =>
  (await textOf("apps-status")) === "agent App cancellation delivered");
await waitFor("cancellation App acknowledgement", async () =>
  (await eventKinds()).includes("host/app_tool_call"));
await waitFor("cancelled native turn", async () =>
  (await textOf("turn-outcome")).includes("turn: cancelled / cancelled"));
await closeAndDisconnect();

const delayRoute = await openAndPrompt({
  fixture: "mcp-apps-delay-v0.1", server: "notes",
  nonce: "browser-teardown-race-0.57", text: "View teardown cannot own this call",
  delay_ms: 3000,
});
await waitFor("pending App", async () => (await textOf("apps-status")) === "app ready");
await click("#app-close");
await waitFor("pending App close", async () => (await textOf("apps-status")) === "app closed");
writeFileSync(delayRelease, "view-closed\n", {flag: "wx"});
await waitFor("native delayed turn", async () =>
  (await textOf("turn-outcome")).includes("turn: end_turn / completed"));
await click("#disconnect");
await waitFor("final disconnect", async () => evaluate(`document.getElementById("disconnect").disabled`));

console.log(JSON.stringify({errorRoute, cancelRoute, delayRoute}));
cleanup(0);
