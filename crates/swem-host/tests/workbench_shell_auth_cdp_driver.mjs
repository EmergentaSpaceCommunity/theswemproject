// Browser driver: a person gets an agent that needs a key signed in, on the
// page, and then talks to it. usage: node <this-file> <browser> <workbench-url>
//
// The steps are the ones the owner could not get past: the Agent space says
// what the agent will ask for before anything starts; a start without the key
// fails in a sentence, not a payload; the key typed under the agent's own
// variable name is enough; the next start reaches a reply.
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [browser, url] = process.argv.slice(2);
if (!browser || !url) {
  console.error("usage: node workbench_shell_auth_cdp_driver.mjs <browser> <url>");
  process.exit(2);
}
const profile = mkdtempSync(join(tmpdir(), "swem-auth-cdp-"));
const sandboxArgs = process.env.SWEM_BROWSER_NO_SANDBOX === "1" ? ["--no-sandbox"] : [];
const child = spawn(browser, [
  "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
  "--window-size=1280,900", `--user-data-dir=${profile}`, "--remote-debugging-port=0",
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
const textAt = (selector) => evaluate(`(document.querySelector(${JSON.stringify(selector)}) || {textContent: ""}).textContent`);
const exists = (selector) => evaluate(`document.querySelector(${JSON.stringify(selector)}) !== null`);
const waitFor = async (label, predicate, attempts = 150, interval = 200) => {
  for (let i = 0; i < attempts; i++) {
    try { if (await predicate()) return; } catch {}
    await sleep(interval);
  }
  let card = "";
  try {
    card = await evaluate(`JSON.stringify((() => {
      const row = document.querySelector('.auth-var[data-var-name="FIXTURE_KEY"]');
      const input = row && row.querySelector("input");
      const save = row && row.querySelector(".auth-var-save");
      const rect = input ? input.getBoundingClientRect() : null;
      return {present: row && row.dataset.present, value: input && input.value, saveDisabled: save && save.disabled,
        error: row && (row.querySelector(".bad") || {}).textContent, inputRect: rect && {top: rect.top, left: rect.left, w: rect.width, h: rect.height},
        active: document.activeElement && (document.activeElement.id || document.activeElement.tagName),
        vault: (document.getElementById("auth-error") || {}).textContent, innerHeight: window.innerHeight,
        fetches: (window.__swemFetchLog || []).slice(-8)};
    })())`);
  } catch (error) { card = String(error); }
  cleanup(2, `timed out waiting for ${label}; access=${await textOf("auth-status")}; problem=${await textOf("close-error")}; phase=${await textOf("phase")}; card=${card}`);
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
// Real typing: focus by click, then insert.
const type = async (selector, text) => {
  await click(selector);
  await send("Input.insertText", {text});
};

await waitFor("production Workbench renderer", async () => evaluate(`
  window.__SWEM_WORKBENCH_WEB__ === "react-19" &&
  document.querySelector('[data-workbench-space="agent"]') !== null
`));
// The page's own host requests, for the failure diagnostics only.
await evaluate(`(() => {
  window.__swemFetchLog = [];
  const original = window.fetch.bind(window);
  window.fetch = async (...args) => {
    const path = String(args[0]);
    const entry = path.includes("wait_ms=") ? null : {method: (args[1] && args[1].method) || "GET", path};
    if (entry) window.__swemFetchLog.push(entry);
    try {
      const response = await original(...args);
      if (entry) entry.status = response.status;
      return response;
    } catch (error) {
      if (entry) entry.error = String(error);
      throw error;
    }
  };
})()`);
// The product lands on the Project space (the ladder is the door); the
// agent's surface is one click away, as a person reaches it.
await waitFor("the space bar", async () => evaluate(`document.getElementById("space-agent") !== null`));
await click("#space-agent");
await waitFor("the agent space", async () =>
  evaluate(`document.querySelector('.app-shell[data-active-space="agent"]') !== null`));
await waitFor("profile loaded", async () =>
  (await evaluate(`document.getElementById("profiles").options.length`)) === 1);
// The keys live in Setup now, with everything else a person gives an agent;
// the rail only says whether a key is in place.
await click('[data-agent-tab="environment"]');
await waitFor("the setup panel", async () =>
  exists('[data-agent-panel="environment"]:not([hidden])'));

// 1. Before anything starts, the page says what the agent will ask for.
const variable = '.auth-var[data-var-name="FIXTURE_KEY"]';
await waitFor("the agent's sign-in method, by name", async () =>
  (await exists('#auth-methods .auth-method[data-method-id="api-key"]')) &&
  (await exists(`${variable}[data-present="false"]`)));
const accessBefore = await textOf("auth-status");

// 2. Starting without the key fails in a sentence.
await click("#open-new");
await waitFor("a sentence about the missing key", async () =>
  (await textOf("close-error")).includes("Authentication required") &&
  (await evaluate(`document.getElementById("auth-card").dataset.needsAuth`)) === "true");
const problem = await textOf("close-error");
if (problem.includes("{") || problem.includes("}")) cleanup(2, `a payload reached the person: ${problem}`);
if ((await textOf("connection-id")) !== "-") cleanup(2, "a connection opened without the key");

// 3. The key, typed under the variable the agent named.
await type(`${variable} input`, "fixture-secret-value");
await click(`${variable} .auth-var-save`);
await waitFor("the key stored", async () =>
  (await exists(`${variable}[data-present="true"]`)) &&
  (await exists('#secret-list .secret-entry[data-name="FIXTURE_KEY"]')));
if ((await evaluate(`document.querySelector(${JSON.stringify(`${variable} input`)}).value`)) !== "") {
  cleanup(2, "the typed value stayed in the field after it was stored");
}

// 4. The next start reaches the agent, and a reply.
await click("#open-new");
await waitFor("session started", async () => (await textOf("connection-id")) !== "-", 300);
const route = await textOf("route");
await waitFor("ready", async () => (await textOf("phase")) === "ready");
if ((await evaluate(`document.getElementById("auth-card").dataset.needsAuth`)) !== "false") {
  cleanup(2, "the card still asks for a sign-in after the session started");
}
// The conversation is its own tab; the keys were on Setup.
await click('[data-agent-tab="conversation"]');
await waitFor("the composer", async () => exists("#prompt-text"));
await type("#prompt-text", "Привет");
await click("#send");
await waitFor("a reply", async () =>
  (await textOf("turn-outcome")).startsWith("turn:") &&
  (await evaluate(`document.querySelectorAll("#conversation .message.assistant .message-body").length`)) >= 1);
const reply = await textAt("#conversation .message.assistant .message-body");
// 5. A key the agent never asked for, added by hand. The form under "Keys on
// this computer" takes a kind, a variable name and a value, and no walk had
// ever pressed it - the steps above go through the row the agent advertised.
await click('[data-agent-tab="environment"]');
await waitFor("the setup panel again", async () => exists('[data-agent-panel="environment"]:not([hidden])'));
await evaluate(`document.querySelector(".vault").open = true`);
await waitFor("the form for a key of one's own", async () => exists("#secret-add #secret-save"));
const vaultField = async (id, text) => {
  await evaluate(`(() => {
    const field = document.querySelector("#secret-add #" + ${JSON.stringify(id)});
    Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set.call(field, ${JSON.stringify(text)});
    field.dispatchEvent(new Event("input", {bubbles: true}));
  })()`);
};
if (!(await exists("#secret-add #secret-name"))) {
  cleanup(2, "the kind chosen by default needs a name and the form does not ask for one");
}

// Blanks are not a value. A key of spaces is handed to the agent as an
// environment variable of spaces, which is worse than not having it.
await vaultField("secret-name", "WALK_BLANK_KEY");
await vaultField("secret-value", "   ");
await click("#secret-add #secret-save");
// Either outcome ends the wait, so the walk reports what happened rather
// than "timed out": the page answers, or the key lands in the list.
await waitFor("the blanks are answered one way or the other", async () =>
  (await textOf("auth-error")).length > 0 ||
  (await exists('#secret-list .secret-entry[data-name="WALK_BLANK_KEY"]')));
const blankAnswer = await textOf("auth-error");
if (await exists('#secret-list .secret-entry[data-name="WALK_BLANK_KEY"]')) {
  cleanup(1, `a key of only blanks was stored, and the page said ${JSON.stringify(blankAnswer)}`);
}
if (!/value/i.test(blankAnswer)) {
  cleanup(1, `a key of only blanks was not refused in words: ${blankAnswer}`);
}

// A name that is not an environment variable name. The field upper-cases
// what is typed, which is not the same as judging it.
await vaultField("secret-name", "WALK-KEY");
await vaultField("secret-value", "walk-value");
await click("#secret-add #secret-save");
await waitFor("the name is answered one way or the other", async () =>
  (await textOf("auth-error")).includes("WALK-KEY") ||
  (await evaluate(`document.querySelectorAll("#secret-list .secret-entry").length`)) > 1);
const nameAnswer = await textOf("auth-error");
if (!/environment variable/i.test(nameAnswer) || !nameAnswer.includes("WALK-KEY")) {
  cleanup(1, `an impossible variable name was not explained: ${JSON.stringify(nameAnswer)}`);
}

// And the ordinary way, which is what the form is for.
await vaultField("secret-name", "WALK_OWN_KEY");
await vaultField("secret-value", "walk-value");
await click("#secret-add #secret-save");
await waitFor("the key of one's own is listed", async () =>
  exists('#secret-list .secret-entry[data-name="WALK_OWN_KEY"]'));
if ((await evaluate(`document.querySelector("#secret-add #secret-value").value`)) !== "") {
  cleanup(2, "the typed value stayed in the field after it was stored");
}

// Forgetting one. The agent's own key must stay where it is.
await click('#secret-list .secret-entry[data-name="WALK_OWN_KEY"] .secret-remove');
await waitFor("the key is forgotten", async () =>
  !(await exists('#secret-list .secret-entry[data-name="WALK_OWN_KEY"]')));
if (!(await exists('#secret-list .secret-entry[data-name="FIXTURE_KEY"]'))) {
  cleanup(1, "forgetting one key took the other with it");
}

await click("#disconnect");
await waitFor("disconnected", async () => evaluate(`document.getElementById("disconnect").disabled`));
if (!(await textOf("terminal")).startsWith("Session ended")) cleanup(2, `the end was not a sentence: ${await textOf("terminal")}`);

console.log(JSON.stringify({route, access_before: accessBefore, problem, reply, blank_answer: blankAnswer, name_answer: nameAnswer}));
cleanup(0);
