// Dependency-free real-browser gate for native ACP v1 form and URL
// elicitation. usage: node <this-file> <browser> <workbench-url>
import {spawn} from "node:child_process";
import {mkdtempSync, readFileSync, rmSync, writeFileSync} from "node:fs";
import {tmpdir} from "node:os";
import {join} from "node:path";

const [browser, url] = process.argv.slice(2);
if (!browser || !url) process.exit(2);
const profile = mkdtempSync(join(tmpdir(), "swem-elicitation-cdp-"));
/// Chromium refuses to start its sandbox as uid 0, which is how the container
/// gates run. This is a property of the machine, not of any one driver, so it
/// is decided here once rather than threaded through ten argument lists.
const sandboxArgs = process.env.SWEM_BROWSER_NO_SANDBOX === "1" ? ["--no-sandbox"] : [];
const child = spawn(browser, [
  "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
  "--window-size=1280,1000", `--user-data-dir=${profile}`, "--remote-debugging-port=0",
  ...sandboxArgs, "about:blank",
], {stdio:"ignore"});
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
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
    try { rmSync(profile, {recursive:true, force:true}); } catch {}
    process.exit(code);
  }, 300);
  throw walkStopped;
};
process.on("uncaughtException", (error) => {
  if (error && error.walkStopped === true) { return; }
  try { cleanup(2, error.stack || String(error)); } catch {}
});
process.on("unhandledRejection", (error) => {
  if (error && error.walkStopped === true) { return; }
  try { cleanup(2, error.stack || String(error)); } catch {}
});

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
    const waiter = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) waiter.reject(new Error(message.error.message));
    else waiter.resolve(message.result);
  }
};
const send = (method, params={}) => new Promise((resolve, reject) => {
  const id = nextId++;
  pending.set(id, {resolve, reject});
  ws.send(JSON.stringify({id, method, params}));
});
await send("Page.enable");
await send("Runtime.enable");
await send("Target.setDiscoverTargets", {discover:true});
await send("Page.navigate", {url});
const evaluate = async (expression) => {
  const result = await send("Runtime.evaluate", {expression, returnByValue:true, awaitPromise:true});
  if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
  return result.result.value;
};
const textOf = (id) => evaluate(
  `(document.getElementById(${JSON.stringify(id)}) || {textContent:""}).textContent`);
const waitFor = async (label, predicate, attempts=200) => {
  for (let i = 0; i < attempts; i++) {
    try { if (await predicate()) return; } catch {}
    await sleep(100);
  }
  let snapshot = "";
  try {
    snapshot = ` context=${await textOf("elicitation-context")} error=${await textOf("elicitation-error")}` +
      ` turn=${await textOf("turn-outcome")} close=${await textOf("close-error")} phase=${await textOf("phase")}`;
    const shot = await send("Page.captureScreenshot", {format: "png"});
    const path = join(process.env.SWEM_FAILURE_DIR || tmpdir(), "gate-failure-elicitation.png");
    writeFileSync(path, Buffer.from(shot.data, "base64"));
    snapshot += ` screenshot=${path} form=${await evaluate(`getComputedStyle(document.getElementById("elicitation")).display`)}` +
      ` events=${await evaluate(`JSON.stringify([...document.querySelectorAll("#events li, .event-row")].slice(-8).map((n) => n.textContent.slice(0, 80)))`)}`;
  } catch {}
  throw new Error(`timed out waiting for ${label};${snapshot}`);
};
const click = async (selector) => {
  // Bring it into view first: a control below the fold of a scrolling rail
  // has a viewport point outside the window, and a click there lands nowhere.
  await evaluate(`document.querySelector(${JSON.stringify(selector)})?.scrollIntoView({block: "center", inline: "center", behavior: "instant"})`);
  // Bring it into view first: a form taller than the window puts its
  // accept button below the fold, where a click lands nowhere.
  await evaluate(`document.querySelector(${JSON.stringify(selector)})?.scrollIntoView({block: "center", inline: "center", behavior: "instant"})`);
  const rect = JSON.parse(await evaluate(
    `JSON.stringify(document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect())`));
  const x = rect.left + rect.width / 2;
  const y = rect.top + rect.height / 2;
  await send("Input.dispatchMouseEvent", {type:"mousePressed", x, y, button:"left", clickCount:1});
  await send("Input.dispatchMouseEvent", {type:"mouseReleased", x, y, button:"left", clickCount:1});
};
const prompt = async (fixture) => {
  await evaluate(`document.getElementById("prompt-text").value = ${JSON.stringify(JSON.stringify({fixture}))}`);
  await click("#send");
};

// The product lands on the Project space (the ladder is the door); the
// agent's surface is one click away, as a person reaches it.
await waitFor("the space bar", async () => evaluate(`document.getElementById("space-agent") !== null`));
await click("#space-agent");
await waitFor("the agent space", async () =>
  evaluate(`document.querySelector('.app-shell[data-active-space="agent"]') !== null`));
await waitFor("profile", async () =>
  (await evaluate(`document.getElementById("profiles").options.length`)) === 1);
await click("#open-new");
await waitFor("request-scoped initialize form", async () =>
  (await textOf("elicitation-context")).includes("request initialize-fixture"));
const initializeContext = await textOf("elicitation-context");
await evaluate(`document.querySelector('[data-name="workspace_label"]').value = "browser-pre-session"`);
await click("#elicitation-accept");
await waitFor("connection", async () => (await textOf("connection-id")) !== "-");
const route = await textOf("route");

await prompt("elicitation-form-v0.1");
await waitFor("ACP form", async () =>
  (await evaluate(`getComputedStyle(document.getElementById("elicitation")).display`)) === "grid");
const formContext = await textOf("elicitation-context");
if (!formContext.includes("ACP agent") || !formContext.includes("session")) {
  cleanup(2, `requesting agent/scope not identified: ${formContext}`);
}
// An answer that leaves out a choice the schema requires cannot be sent. The
// several-choice field carries the schema's requirement as the browser's own,
// so pressing Accept with nothing chosen does not submit the form: the person
// is held on the dialog by the field itself rather than by a message of ours.
// Measured rather than assumed - the page shows nothing in
// `#elicitation-error` here, because the form never gets as far as our code.
await evaluate(`(() => {
  const fields = document.getElementById("elicitation-fields");
  fields.querySelector('[data-name="strategy"]').value = "bold";
  fields.querySelector('[data-name="iterations"]').value = "3";
  const stems = fields.querySelector('[data-name="stems"]');
  for (const option of stems.options) option.selected = false;
})()`);
const emptyChoice = JSON.parse(await evaluate(`JSON.stringify((() => {
  const stems = document.querySelector('#elicitation-fields [data-name="stems"]');
  return {required: stems.required, valid: stems.checkValidity(), selected: stems.selectedOptions.length};
})())`));
if (!emptyChoice.required || emptyChoice.valid || emptyChoice.selected !== 0) {
  cleanup(1, `the several-choice field does not hold a person to the schema: ${JSON.stringify(emptyChoice)}`);
}
await click("#elicitation-accept");
await sleep(500);
if (!(await evaluate(`document.getElementById("elicitation").style.display !== "none"`))) {
  cleanup(1, "an answer without a required choice closed the dialog");
}
if ((await textOf("turn-outcome")).startsWith("turn:")) {
  cleanup(1, "an answer without a required choice was sent");
}

await evaluate(`(() => {
  const fields = document.getElementById("elicitation-fields");
  fields.querySelector('[data-name="strategy"]').value = "bold";
  fields.querySelector('[data-name="iterations"]').value = "3";
  fields.querySelector('[data-name="gain_db"]').value = "-1.25";
  fields.querySelector('[data-name="normalize"]').value = "true";
  const stems = fields.querySelector('[data-name="stems"]');
  for (const option of stems.options) option.selected = ["voice", "fx"].includes(option.value);
})()`);
await click("#elicitation-accept");
await waitFor("form response", async () => (await textOf("turn-outcome")).startsWith("turn:"));

await prompt("elicitation-url-v0.1");
await waitFor("URL consent", async () => (await textOf("elicitation-context")).includes("target host"));
const urlContext = await textOf("elicitation-context");
if (!urlContext.includes("example.invalid") || !urlContext.includes("full URL")) {
  cleanup(2, `URL host/full target absent: ${urlContext}`);
}
const before = (await send("Target.getTargets")).targetInfos.filter((entry) => entry.type === "page").length;
await click("#elicitation-accept");
await waitFor("consented URL target", async () => {
  const targets = (await send("Target.getTargets")).targetInfos;
  return targets.some((entry) => entry.url.startsWith("https://example.invalid/connect"));
});
const after = (await send("Target.getTargets")).targetInfos.filter((entry) => entry.type === "page").length;
if (after <= before) cleanup(2, "URL target existed before explicit consent or did not open");
await waitFor("URL response", async () => (await textOf("turn-outcome")).startsWith("turn:"));
const connection = await textOf("connection-id");
console.log(JSON.stringify({connection, route, initialize_context:initializeContext, form_context:formContext, url_context:urlContext, pages_before:before, pages_after:after}));
cleanup(0);
