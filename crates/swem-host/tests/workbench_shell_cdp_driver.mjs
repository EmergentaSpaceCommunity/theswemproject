// Dependency-free Chrome DevTools Protocol driver for the generic Workbench
// shell gate (Node >= 22: built-in WebSocket, fetch).
//
// usage: node workbench_shell_cdp_driver.mjs <browser exe> <url>
//
// Drives a REAL headless browser through the whole acceptance list on the
// arbitrary echo fixture profile: open new connection, late prompts, ordered
// stream visible in the DOM, interactive permission allow (real click on the
// exact agent-offered option), active-turn cancel, disconnect, resume on the
// same route with no re-delivery of acknowledged events, and the fail-closed
// unadvertised close. Button presses and typing are real Input events; the
// only JS shortcuts are reading DOM state and setting form field values.
// The truth oracle lives in the Rust test (routing ledger + MCP receipt);
// this driver reports the identities it observed as one JSON line.
import {spawn} from "node:child_process";
import {mkdtempSync, readFileSync, rmSync, writeFileSync} from "node:fs";
import {tmpdir} from "node:os";
import {join} from "node:path";

const [browser, url, mode, marker] = process.argv.slice(2);
if (!browser || !url || (mode && !["--simple", "--onboarding"].includes(mode))) {
  console.error("usage: node workbench_shell_cdp_driver.mjs <browser> <url> [--simple|--onboarding <marker>]");
  process.exit(2);
}
const profile = mkdtempSync(join(tmpdir(), "swem-shell-cdp-"));
/// Chromium refuses to start its sandbox as uid 0, which is how the container
/// gates run. This is a property of the machine, not of any one driver, so it
/// is decided here once rather than threaded through ten argument lists.
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
// What the composer's five ACP resource fields are given, named here so the
// walk and the report agree with what the Rust side looks for in the ledger.
const RESOURCE_LINK_URI = "file:///tmp/swem-walk-resource-link.txt";
const RESOURCE_LINK_NAME = "SWEM_WALK_LINK_NAME";
const RESOURCE_EMBED_URI = "file:///tmp/swem-walk-embedded.txt";
const RESOURCE_EMBED_BODY = "SWEM_WALK_EMBEDDED_BODY";
const RESOURCE_ORPHAN_BODY = "SWEM_WALK_ORPHAN_BODY";
// Who is writing, typed into the composer. ACP carries no author, so this
// is the only thing that tells an agent - or a person reading the lane back -
// which of two people at this browser wrote a turn.
const WRITING_AS = "Ada Lovelace";

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

// A timed-out step leaves the pixels and the scroll geometry of the product
// panels behind: a click that missed a scrolled-away control is only
// diagnosable from the real frame, never from the driver's own arithmetic.
let lastClick = null;
const failure = async (label) => {
  const diagnostics = {label, lastClick};
  try {
    diagnostics.geometry = await evaluate(`JSON.stringify((() => {
      const box = (selector) => {
        const node = document.querySelector(selector);
        if (!node) return null;
        const rect = node.getBoundingClientRect();
        return {top: rect.top, left: rect.left, width: rect.width, height: rect.height,
          scrollTop: node.scrollTop, scrollHeight: node.scrollHeight, clientHeight: node.clientHeight};
      };
      return {rail: box(".rail"), main: box(".main"), context: box(".context"), space: box(".agent-space"),
        disconnect: box("#disconnect"), page: {innerHeight: window.innerHeight, innerWidth: window.innerWidth},
        phase: (document.getElementById("phase") || {}).textContent,
        turnOutcome: (document.getElementById("turn-outcome") || {}).textContent,
        prompt: (document.getElementById("prompt-text") || {}).value,
        said: [...document.querySelectorAll("#conversation .message .message-body")].map((node) => node.textContent).slice(-3),
        active: document.activeElement && document.activeElement.id};
    })())`);
  } catch (error) { diagnostics.geometry = `geometry failed: ${error.message || error}`; }
  // What the page itself saw: its own host requests (method, path, status,
  // elapsed ms; long-polls excluded) and the error texts it renders.
  try {
    diagnostics.page = JSON.parse(await evaluate(`JSON.stringify({
      fetchLog: (window.__swemFetchLog || []).slice(-40),
      closeError: (document.getElementById("close-error") || {}).textContent,
      terminal: (document.getElementById("terminal") || {}).textContent,
      appsStatus: (document.getElementById("apps-status") || {}).textContent,
    })`));
  } catch (error) { diagnostics.page = `page diagnostics failed: ${error.message || error}`; }
  try {
    const shot = await send("Page.captureScreenshot", {format: "png"});
    diagnostics.screenshot = process.env.SWEM_SHELL_FAILURE_SCREENSHOT || join(tmpdir(), "swem-shell-failure.png");
    writeFileSync(diagnostics.screenshot, Buffer.from(shot.data, "base64"));
  } catch (error) { diagnostics.screenshot = `screenshot failed: ${error.message || error}`; }
  cleanup(2, `timed out waiting for ${label}; diagnostics=${JSON.stringify(diagnostics)}`);
};
const waitFor = async (label, predicate, attempts = 150, interval = 200) => {
  for (let i = 0; i < attempts; i++) {
    try { if (await predicate()) return; } catch {}
    await sleep(interval);
  }
  await failure(label);
};

// The product shell must come from the production React asset embedded by the
// Rust host; a stale static fallback would make later Workbench composition
// impossible while still allowing most legacy DOM assertions to pass.
await waitFor("production Workbench renderer", async () => evaluate(`
  window.__SWEM_WORKBENCH_WEB__ === "react-19" &&
  document.getElementById("workbench-root")?.dataset.renderer === "react" &&
  document.querySelector('[data-workbench-space="agent"]') !== null
`));
// Observe the page's own host requests for the failure diagnostics; the
// page's behaviour is untouched (the wrapper only records and forwards).
await evaluate(`(() => {
  if (window.__swemFetchLog) return;
  window.__swemFetchLog = [];
  const original = window.fetch.bind(window);
  window.fetch = async (...args) => {
    const path = String(args[0]);
    const method = (args[1] && args[1].method) || "GET";
    const started = Date.now();
    const entry = path.includes("wait_ms=") ? null : {method, path, started: started % 1000000};
    if (entry) window.__swemFetchLog.push(entry);
    try {
      const response = await original(...args);
      if (entry) { entry.status = response.status; entry.ms = Date.now() - started; }
      return response;
    } catch (error) {
      if (entry) { entry.error = String(error); entry.ms = Date.now() - started; }
      throw error;
    }
  };
})()`);

// A real click at the element's viewport centre.
const click = async (selector) => {
  // Bring it into view first: a control below the fold of a scrolling rail
  // has a viewport point outside the window, and a click there lands nowhere.
  await evaluate(`document.querySelector(${JSON.stringify(selector)})?.scrollIntoView({block: "center", inline: "center", behavior: "instant"})`);
  const rect = JSON.parse(await evaluate(
    `JSON.stringify(document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect())`));
  const x = rect.left + rect.width / 2;
  const y = rect.top + rect.height / 2;
  lastClick = {selector, x, y, rect};
  await send("Input.dispatchMouseEvent", {type: "mouseMoved", x, y, pointerType: "mouse"});
  await send("Input.dispatchMouseEvent", {type: "mousePressed", x, y, button: "left", buttons: 1, clickCount: 1, pointerType: "mouse"});
  await send("Input.dispatchMouseEvent", {type: "mouseReleased", x, y, button: "left", clickCount: 1, pointerType: "mouse"});
};

// The product lands on the Project space (the ladder is the door); every
// mode of this driver works in the agent's surface, one click away.
await click("#space-agent");
await waitFor("the agent space", async () =>
  evaluate(`document.querySelector('.app-shell[data-active-space="agent"]') !== null`));

// Real typing into the prompt textarea: focus by click, clear, insert text.
const typePrompt = async (text) => {
  // Real typing into the real textarea, and then a check that it landed: a
  // click that misses focus used to leave the box empty, send an empty
  // prompt, and hang the gate on a turn that had already finished.
  for (let attempt = 0; attempt < 5; attempt += 1) {
    await evaluate(`document.getElementById("prompt-text").value = ""`);
    await click("#prompt-text");
    await send("Input.insertText", {text});
    const landed = await evaluate(`document.getElementById("prompt-text").value`);
    if (landed === text) return;
    await sleep(200);
  }
  await failure(`typing into the composer never landed: ${text.slice(0, 60)}`);
};

// What the LIVE lane delivered to this surface. A resumed session also shows
// what was said in it before, read from the route's history on purpose; those
// rows are marked, and they are not deliveries.
const eventSequences = () => evaluate(
  `JSON.stringify([...document.querySelectorAll("#events li:not([data-history])")].map((li) => ({
     kind: li.dataset.kind, sequence: Number(li.dataset.sequence) })))`)
  .then((value) => JSON.parse(value));

// Everything the person can see in the lane, history included.
const shownSequences = () => evaluate(
  `JSON.stringify([...document.querySelectorAll("#events li")].map((li) => Number(li.dataset.sequence)))`)
  .then((value) => JSON.parse(value));

// --- Simple mode: one late prompt on the sole profile (live adapters) -----
if (mode === "--simple") {
  // The product lands on the Project space (the ladder is the door); the
  // agent's surface is one click away, as a person reaches it.
  await waitFor("the space bar", async () => evaluate(`document.getElementById("space-agent") !== null`));
  await click("#space-agent");
  await waitFor("the agent space", async () =>
    evaluate(`document.querySelector('.app-shell[data-active-space="agent"]') !== null`));
  await waitFor("profiles loaded", async () =>
    (await evaluate(`document.getElementById("profiles").options.length`)) >= 1);
  await click("#open-new");
  await waitFor("connection open", async () => (await textOf("connection-id")) !== "-", 300);
  const route = await textOf("route");
  await typePrompt(marker || "workbench shell live marker");
  await click("#send");
  await waitFor("live turn outcome", async () =>
    (await textOf("turn-outcome")).startsWith("turn:"), 600);
  await click("#disconnect");
  await waitFor("disconnected", async () =>
    evaluate(`document.getElementById("disconnect").disabled`), 300);
  console.log(JSON.stringify({main_route: route}));
  cleanup(0);
  await sleep(1000);
}

// --- Product first run: discovered agent -> profile -> native chat --------
if (mode === "--onboarding") {
  const expected = "SWEM_CONTENT_MATRIX " + (marker || "product onboarding marker");
  await waitFor("discovered agent offered", async () =>
    (await evaluate(`document.querySelectorAll(".create-agent:not(:disabled)").length`)) === 1);
  await click(".create-agent:not(:disabled)");
  await waitFor("profile created", async () =>
    (await evaluate(`document.getElementById("profiles").options.length`)) === 1);
  await click("#open-new");
  await waitFor("first connection open", async () => (await textOf("connection-id")) !== "-", 300);
  const route = await textOf("route");
  // The model over the conversation is the agent's own: the echo fixture
  // offers `fast` and `quality` and starts on `fast`, and the page reads
  // that from the session rather than inventing a list.
  await waitFor("the session's model is read off the agent", async () =>
    (await evaluate(`document.getElementById("session-model").value`)) === "fast", 300);
  const attachment = join(profile, "product-brief.txt");
  writeFileSync(attachment, "attachment-evidence", "utf8");
  await send("DOM.enable");
  const document = await send("DOM.getDocument");
  const fileInput = await send("DOM.querySelector", {
    nodeId: document.root.nodeId, selector: "#file-input"});
  await send("DOM.setFileInputFiles", {nodeId: fileInput.nodeId, files: [attachment]});
  await waitFor("attachment selected", async () =>
    (await textOf("attachment-list")).includes("product-brief.txt"));
  await typePrompt(expected);
  await click("#send");
  await waitFor("rendered user and agent messages", async () => evaluate(`
    [...document.querySelectorAll("#conversation .message.user .message-body")]
      .some((node) => node.textContent.includes(${JSON.stringify(expected)})) &&
    document.querySelectorAll("#conversation .message.assistant .message-body").length > 0
  `), 300);
  await waitFor("first turn completed", async () =>
    (await textOf("turn-outcome")).startsWith("turn:"), 300);
  await waitFor("agent output artifact", async () =>
    (await evaluate(`document.querySelectorAll(".artifact-card").length`)) > 0, 300);
  const artifactProof = JSON.parse(await evaluate(`(async () => {
    const card = document.querySelector(".artifact-card");
    const id = card.dataset.descriptorId;
    const source = "/api/content/" + encodeURIComponent(id);
    const full = await fetch(source);
    const fullBytes = [...new Uint8Array(await full.arrayBuffer())];
    const partial = await fetch(source, {headers: {range: "bytes=0-9"}});
    const partialBytes = [...new Uint8Array(await partial.arrayBuffer())];
    return JSON.stringify({
      descriptor_id: id,
      full_status: full.status,
      full_bytes: fullBytes,
      representation_digest: full.headers.get("repr-digest"),
      partial_status: partial.status,
      partial_range: partial.headers.get("content-range"),
      partial_bytes: partialBytes,
    });
  })()`));
  await evaluate(`document.getElementById("prompt-text").value = "SWEM_WORKSPACE_LINK"`);
  await click("#send");
  await waitFor("linked turn completed", async () =>
    (await textOf("turn-outcome")).startsWith("turn:"), 300);
  await waitFor("linked workspace artifact", async () =>
    (await evaluate(`document.querySelectorAll('.artifact-card[data-artifact-name="linked-output.bin"]').length`)) === 1,
    300);
  await waitFor("unavailable linked resources", async () =>
    (await evaluate(`document.querySelectorAll('.artifact-card.unavailable[data-reason="resource_link_unavailable"]').length`)) === 2,
    300);
  const linkedArtifactProof = JSON.parse(await evaluate(`(async () => {
    const card = document.querySelector('.artifact-card[data-artifact-name="linked-output.bin"]');
    const id = card.dataset.descriptorId;
    const source = "/api/content/" + encodeURIComponent(id);
    const full = await fetch(source);
    return JSON.stringify({
      descriptor_id: id,
      full_status: full.status,
      full_bytes: [...new Uint8Array(await full.arrayBuffer())],
      unavailable_count: document.querySelectorAll(
        '.artifact-card.unavailable[data-reason="resource_link_unavailable"]'
      ).length,
    });
  })()`));
  // Who is writing. The field had never been typed into by any walk, so
  // nothing said whether a name reaches the agent or the lane at all. It is
  // committed when the field is left, which is what happens when a person
  // clicks into the message box, so that is what this does.
  await evaluate(`(() => {
    const field = document.getElementById("writing-as");
    field.focus();
    field.value = ${JSON.stringify(WRITING_AS)};
    field.blur();
  })()`);
  await typePrompt("SWEM_WRITTEN_BY_A_PERSON");
  await click("#send");
  await waitFor("the named turn ends", async () =>
    (await textOf("turn-outcome")).startsWith("turn:"), 300);
  await click("#disconnect");
  await waitFor("first connection disconnected", async () =>
    evaluate(`document.getElementById("disconnect").disabled`), 300);

  // The theme, which a person picks from the bar at the top and no walk had
  // ever touched: the control carries no name of its own, only an aria
  // label, so it sat among the thirteen this project counts but cannot
  // judge. What a person chooses about how the product looks is
  // the same kind of choice as which space they were in and who is writing,
  // and both of those come back after a reload.
  const themePicked = await evaluate(`(() => {
    const picker = document.getElementById("theme");
    if (!picker) return "missing";
    const setter = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value').set;
    setter.call(picker, "light");
    picker.dispatchEvent(new Event('change', {bubbles: true}));
    return "ok";
  })()`);
  if (themePicked !== "ok") cleanup(2, "there is nowhere to pick the theme");
  await waitFor("the product is light", async () =>
    (await evaluate(`document.documentElement.dataset.theme`)) === "light");

  await send("Page.reload", {ignoreCache: true});
  await waitFor("remembered profile after reload", async () =>
    (await evaluate(`document.getElementById("profiles").value`)) === "swem-echo-agent");
  // And it is still light. The space a person chose is kept in this
  // browser on purpose - "a person who chose the Agent space keeps it" -
  // and the theme sat on the same bar keeping nothing.
  const themeAfter = await evaluate(`document.documentElement.dataset.theme`);
  const themeShown = await evaluate(`document.getElementById("theme").value`);
  if (themeAfter !== "light" || themeShown !== "light") {
    cleanup(1, `after a reload the product forgot the theme a person picked: html=${JSON.stringify(themeAfter)} picker=${JSON.stringify(themeShown)}`);
  }
  await click("#open-resume");
  await waitFor("remembered route resumed", async () =>
    (await textOf("route")) === route && (await textOf("connection-id")) !== "-", 300);
  // And this browser still knows who is writing, so the turn below goes out
  // under the same name without the person typing it again.
  const remembersWho = await evaluate(`document.getElementById("writing-as").value`);
  if (remembersWho !== WRITING_AS) {
    cleanup(1, `after a reload the browser forgot who is writing: ${JSON.stringify(remembersWho)}`);
  }
  await typePrompt("after product reload");
  await click("#send");
  await waitFor("continued after reload", async () =>
    (await textOf("turn-outcome")).startsWith("turn:"), 300);

  // --- The composer's ACP resource fields ---------------------------------
  // Five fields under "ACP resource input" that no walk had ever typed into.
  // Three questions, none of them asked before: does what a person fills
  // reach the agent, does it stop riding the turns after it, and what happens
  // to a body typed without the URI it belongs to.
  const fill = (values) => evaluate(`(() => {
    for (const [id, value] of Object.entries(${JSON.stringify(values)})) {
      document.getElementById(id).value = value;
    }
  })()`);
  await fill({
    "link-uri": RESOURCE_LINK_URI,
    "link-name": RESOURCE_LINK_NAME,
    "link-mime": "text/plain",
    "embed-uri": RESOURCE_EMBED_URI,
    "embed-text": RESOURCE_EMBED_BODY,
  });
  await typePrompt("SWEM_RESOURCE_FIELDS");
  await click("#send");
  await waitFor("the resource turn ends", async () =>
    (await textOf("turn-outcome")).startsWith("turn:"), 300);

  // The next message, with nothing touched in between. The composer clears
  // the text and the files it sent; these five stayed filled, so one resource
  // link went out again with every later message.
  await typePrompt("SWEM_AFTER_RESOURCE_FIELDS");
  await click("#send");
  await waitFor("the plain turn ends", async () =>
    (await textOf("turn-outcome")).startsWith("turn:"), 300);

  // A body with no URI to hang it on. The person typed it, so the product
  // owes them an answer rather than a turn that quietly leaves it out.
  await fill({"link-uri": "", "link-name": "", "link-mime": "", "embed-uri": "", "embed-text": RESOURCE_ORPHAN_BODY});
  await typePrompt("SWEM_ORPHAN_BODY");
  await click("#send");
  await waitFor("the orphan body is answered", async () =>
    (await textOf("turn-outcome")).startsWith("turn failed:"), 300);
  const orphanAnswer = await textOf("turn-outcome");
  if (!/URI/i.test(orphanAnswer)) {
    cleanup(1, `a body typed without its URI was not answered in words: ${orphanAnswer}`);
  }
  await fill({"embed-text": ""});

  await click("#disconnect");
  await waitFor("resumed product connection disconnected", async () =>
    evaluate(`document.getElementById("disconnect").disabled`), 300);
  console.log(JSON.stringify({
    main_route: route,
    artifact_proof: artifactProof,
    linked_artifact_proof: linkedArtifactProof,
    writing_as: WRITING_AS,
    resource_fields: {
      link_uri: RESOURCE_LINK_URI,
      link_name: RESOURCE_LINK_NAME,
      embed_uri: RESOURCE_EMBED_URI,
      embed_body: RESOURCE_EMBED_BODY,
      orphan_body: RESOURCE_ORPHAN_BODY,
      orphan_answer: orphanAnswer,
    },
  }));
  cleanup(0);
  await sleep(1000);
}

// --- Stage 1: new connection on the main profile -------------------------
await waitFor("profiles loaded", async () =>
  (await evaluate(`document.getElementById("profiles").options.length`)) >= 2);
await evaluate(`document.getElementById("profiles").value = "echo-main"`);
await click("#open-new");
// Sixty seconds for the handshake, as every other walk that waits on this
// step: thirty is not enough under a full gate.
await waitFor("connection open", async () => (await textOf("connection-id")) !== "-", 300);
const mainRoute = await textOf("route");

// Late first prompt after the connection was already open.
await typePrompt("late first from the browser");
await click("#send");
await waitFor("first turn outcome", async () =>
  (await textOf("turn-outcome")).startsWith("turn:"));
// Ordered stream is visible in the DOM: submitted < update < response.
await waitFor("ordered stream rendered", async () => {
  const events = await eventSequences();
  const submitted = events.find((event) => event.kind === "host/prompt_submitted");
  const update = events.find((event) => event.kind === "acp/session_update");
  const response = events.find((event) => event.kind === "acp/prompt_response");
  return submitted && update && response &&
    submitted.sequence < update.sequence && update.sequence < response.sequence;
});

// Late second prompt drives a REAL ACP permission request whose allow leads
// to an actual MCP tool call with an out-of-band receipt.
await typePrompt(JSON.stringify({
  fixture: "mcp-echo-permission-v0.1", server: "echo", nonce: "browser-nonce"}));
await click("#send");
await waitFor("permission offered", async () =>
  (await evaluate(`document.querySelectorAll("#permission-options .permission-option").length`)) > 0);
// Lose both the visible question and the HTTP prompt caller. Resume must
// attach to the SAME live runtime, not start session/resume on a second agent.
const pendingConnection = await textOf("connection-id");
const pendingTitle = await textOf("permission-title");
const pendingSequence = await evaluate(`document.getElementById("permission")?.dataset.sequence ?? ""`);
await send("Page.reload", {ignoreCache: true});
await waitFor("profile restored during pending permission", async () =>
  (await evaluate(`document.getElementById("profiles")?.value`)) === "echo-main");
await click("#open-resume");
await waitFor("same runtime reattached", async () =>
  (await textOf("connection-id")) === pendingConnection);
await waitFor("same permission recovered", async () =>
  (await textOf("permission-title")) === pendingTitle &&
  (await evaluate(`document.getElementById("permission")?.dataset.sequence ?? ""`)) === pendingSequence);
const allowSelector = `#permission-options .permission-option[data-kind="allow_once"]`;
await waitFor("allow option present", async () =>
  (await evaluate(`document.querySelectorAll(${JSON.stringify(allowSelector)}).length`)) > 0);
await click(allowSelector);
await waitFor("permitted turn completed", async () =>
  (await textOf("turn-outcome")).includes("completed"));
// The tool the agent asked leave to use is a card with its status, not a
// line of text, and the card the call opened is the one its update closed.
await waitFor("the permitted tool call is a completed card", async () =>
  (await evaluate(`document.querySelectorAll(
    '#conversation .tool-card[data-tool-call-id][data-status="completed"]').length`)) >= 1 &&
  (await evaluate(`document.querySelectorAll(
    '#conversation .tool-card[data-status="pending"], #conversation .tool-card[data-status="in_progress"]').length`)) === 0, 100);

// Active-turn cancel: the fixture blocks until session/cancel arrives, so the
// cancel button is pressed until the turn actually reports cancelled.
await typePrompt(JSON.stringify({fixture: "cancel-v0.1"}));
await click("#send");
await waitFor("turn cancelled", async () => {
  const outcome = await textOf("turn-outcome");
  if (outcome.includes("cancelled")) return true;
  await click("#cancel");
  return false;
});

await click("#disconnect");
await waitFor("disconnected", async () =>
  evaluate(`document.getElementById("disconnect").disabled`));
const beforeResume = await eventSequences();
const maxSequenceBeforeResume = Math.max(0, ...beforeResume.map((event) => event.sequence));

// --- Stage 2: resume the same route ---------------------------------------
await click("#open-resume");
await waitFor("resumed connection open", async () =>
  (await textOf("route")) === mainRoute && (await textOf("connection-id")) !== "-");
await waitFor("resume visible on the lane", async () => {
  const events = await eventSequences();
  return events.some((event) => event.kind === "acp/session_resume");
});
const afterResume = await eventSequences();
const minSequenceAfterResume = Math.min(...afterResume.map((event) => event.sequence));
if (minSequenceAfterResume <= maxSequenceBeforeResume) {
  cleanup(2, `acknowledged events were re-delivered after resume: ` +
    `${minSequenceAfterResume} <= ${maxSequenceBeforeResume}`);
}
// The resumed session opens on what was said in it: the history is there,
// and it reaches back before this surface's cursor.
const shownAfterResume = await shownSequences();
if (!shownAfterResume.some((sequence) => sequence <= maxSequenceBeforeResume)) {
  cleanup(2, `the resumed session shows nothing that was said before: ${JSON.stringify(shownAfterResume)}`);
}
await typePrompt("after resume");
await click("#send");
await waitFor("resumed turn outcome", async () =>
  (await textOf("turn-outcome")).startsWith("turn:"));
await click("#disconnect");
await waitFor("resumed connection disconnected", async () =>
  evaluate(`document.getElementById("disconnect").disabled`));

// --- Stage 2b: the other way back in ---------------------------------------
// Resume reattaches this host to a session the agent is still holding. Load
// is the other door: the agent is asked to hand the conversation back, which
// is what a person needs once the agent's own process has gone. The button
// had been on the rail since the rail existed and no walk had ever pressed
// it, so nothing said this product can do it at all.
await click("#open-load");
await waitFor("loaded connection open", async () =>
  (await textOf("route")) === mainRoute && (await textOf("connection-id")) !== "-", 300);
await waitFor("the load visible on the lane", async () => {
  const events = await eventSequences();
  return events.some((event) => event.kind === "acp/session_load");
});
// And what comes back is the conversation, not an empty one wearing its name.
const shownAfterLoad = await shownSequences();
if (!shownAfterLoad.some((sequence) => sequence <= maxSequenceBeforeResume)) {
  cleanup(1, `the loaded session shows nothing that was said before: ${JSON.stringify(shownAfterLoad)}`);
}
// And it is usable: the lane grows by this turn rather than by nothing.
const shownBeforeLoadedTurn = shownAfterLoad.length;
await typePrompt("after load");
await click("#send");
await waitFor("the loaded session takes a turn", async () =>
  (await shownSequences()).length > shownBeforeLoadedTurn, 300);
await click("#disconnect");
await waitFor("loaded connection disconnected", async () =>
  evaluate(`document.getElementById("disconnect").disabled`));

// --- Stage 3: unadvertised close fails closed ------------------------------
await evaluate(`document.getElementById("profiles").value = "echo-nocls"`);
await evaluate(`document.getElementById("route-id").value = ""`);
await click("#open-new");
await waitFor("close-hiding connection open", async () =>
  (await textOf("route")) !== mainRoute && (await textOf("connection-id")) !== "-");
const noclsRoute = await textOf("route");
await click("#close");
await waitFor("close refused", async () =>
  (await textOf("close-error")).includes("close refused"));
await typePrompt("still usable after refused close");
await click("#send");
await waitFor("post-refusal turn outcome", async () =>
  (await textOf("turn-outcome")).startsWith("turn:"));
await click("#disconnect");
await waitFor("close-hiding connection disconnected", async () =>
  evaluate(`document.getElementById("disconnect").disabled`));

console.log(JSON.stringify({
  main_route: mainRoute,
  nocls_route: noclsRoute,
  max_sequence_before_resume: maxSequenceBeforeResume,
  min_sequence_after_resume: minSequenceAfterResume,
}));
cleanup(0);
