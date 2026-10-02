// Shared Chrome DevTools Protocol helper for the browser gates (Node >= 22:
// built-in WebSocket, fetch): one headless browser with its CDP session and
// the DOM/input helpers the drivers use. Clicks and typing are real Input
// events; the JS shortcuts only read DOM state.
import {spawn} from "node:child_process";
import {existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync} from "node:fs";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {assertHandshakeOrder, handshakeViolations} from "./app_handshake.mjs";

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/// Process-wide registry so a failure can kill every browser and remove
/// every profile before exiting.
// `awaitingEpitaph` is set when a browser's socket closed before its process
// reported a status: the status is worth more than the third of a second it
// costs to wait for it, and without the wait this driver exits first
// and the line is never printed.
// `consoles` holds every launched browser's collected page console, so a
// walk that is about to report success can be asked one last question
// about what the page said while it ran.
export const registry = {children: [], profiles: [], consoles: [], awaitingEpitaph: false, askedToStop: false};

/// Thrown by `cleanup` so the driver stops where it failed.
///
/// `cleanup` used to arm `process.exit` on a timer and *return*, so the caller
/// carried on and printed steps it never took: a walk could shout "not
/// refused" and report "OK" in the same breath, and the exit code that decided
/// it survived only because one timer was longer than the other. The
/// throw ends the walk at its first verdict; the latch below makes that
/// verdict final, so a later `cleanup(0, ...)` cannot lower it and a later
/// failure cannot bury the first cause.
///
/// It is a plain object rather than an `Error` because nothing should print
/// it: the message went out when the verdict was reached.
const walkStopped = {walkStopped: true};
const stoppedAlready = (error) => Boolean(error) && error.walkStopped === true;

export const cleanup = (code, message) => {
  // The first verdict is final. Re-entry is ordinary: a driver's own
  // `catch` may call `cleanup` again with the sentinel it just caught, and a
  // predicate that failed may still reach the walk's closing line.
  if (registry.askedToStop) { throw walkStopped; }
  // A walk only earns its exit code if nothing raced the Apps handshake
  // while it ran: the bridge serves a pre-handshake call rather than
  // dropping it, so a green walk can be hiding one. Asked only on the way
  // out with success, because a walk that is already failing has a cause
  // to report and this would bury it.
  if (code === 0) {
    for (const collected of registry.consoles) {
      assertHandshakeOrder(collected, "this walk", (complaint) => { code = 2; message = complaint; });
      if (code !== 0) break;
    }
  }
  if (message) console.error(message);
  registry.askedToStop = true;
  for (const child of registry.children) { try { child.kill(); } catch {} }
  setTimeout(() => {
    for (const profile of registry.profiles) { try { rmSync(profile, {recursive: true, force: true}); } catch {} }
    process.exit(code);
  }, registry.awaitingEpitaph ? 1_500 : 300);
  throw walkStopped;
};
// A stopped walk reaches these handlers on its way out - the throw above
// escapes the driver's top level - and has already said everything it has to
// say. Anything else is a real fault and still earns a verdict.
process.on("uncaughtException", (error) => {
  if (stoppedAlready(error)) { return; }
  try { cleanup(2, `driver error: ${error.stack || error}`); } catch {}
});
process.on("unhandledRejection", (error) => {
  if (stoppedAlready(error)) { return; }
  try { cleanup(2, `driver rejection: ${error.stack || error}`); } catch {}
});

/// Chromium refuses to start its sandbox as uid 0, which is how the container
/// gates run. This is a property of the machine, not of any one driver, so it
/// is decided here once rather than threaded through ten argument lists.
/// The browser a walk runs: `SWEM_BROWSER`, else the places a Chromium-family
/// browser lives on the three platforms. The Rust side of a walk sets the
/// variable; this is for a driver run by hand.
export function browserPath() {
  if (process.env.SWEM_BROWSER) return process.env.SWEM_BROWSER;
  const candidates = [
    "/opt/pw-browsers/chromium",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/usr/bin/google-chrome",
    "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  ];
  return candidates.find((candidate) => existsSync(candidate)) ?? candidates[0];
}

const sandboxArgs = process.env.SWEM_BROWSER_NO_SANDBOX === "1" ? ["--no-sandbox"] : [];

// The failure screenshot goes where this machine keeps temporary files unless
// SWEM_FAILURE_DIR says otherwise: a literal "/tmp" does not exist on Windows,
// and a screenshot that cannot be written is a failure with no evidence.
// `door`: the page is served at an address and opens on its door, where
// there is no rail until somebody came in.
export async function launchBrowser({browser, url, label, failureDir = tmpdir(), extraArgs = [], door = false}) {
  const profile = mkdtempSync(join(tmpdir(), `swem-cdp-${label}-`));
  registry.profiles.push(profile);
  // The browser's own stderr is kept, not discarded. A browser that dies
  // takes its reason with it - a missing library, a profile it cannot lock,
  // shared memory it cannot map - and with `stdio: "ignore"` that reason went
  // nowhere. What was left was "browser session closed", which names the
  // symptom and nothing else; two gates were spent on that message before
  // anyone noticed it could not say more. Only the tail is kept, because
  // Chromium is talkative and only its last words matter here.
  const child = spawn(browser, [
    "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
    "--window-size=1800,1000", `--user-data-dir=${profile}`, "--remote-debugging-port=0",
    ...sandboxArgs, ...extraArgs, "about:blank",
  ], {stdio: ["ignore", "ignore", "pipe"]});
  registry.children.push(child);
  const said = [];
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    for (const line of String(chunk).split("\n")) {
      if (line.trim().length > 0) said.push(line.trim());
    }
    if (said.length > 60) said.splice(0, said.length - 60);
  });
  /// Why the browser is gone, as far as anything here can tell: how it ended
  /// and the last thing it said.
  ///
  /// The exit is read off the child rather than remembered from its `exit`
  /// event, because the socket closes first: by the time this is asked, the
  /// event may not have run yet, and a status that arrives after the failure
  /// message is no use to anyone.
  const browserEpitaph = () => {
    const parts = [];
    if (child.signalCode) parts.push(`killed by ${child.signalCode}`);
    else if (child.exitCode !== null) parts.push(`exited with ${child.exitCode}`);
    // Not "still running": the socket closes before the child's `exit` event
    // runs, so at this moment there is no status rather than a live process.
    // Read as a fact it sends the next reader looking for a browser that is
    // fine, which is how a SIGKILLed one reports here too.
    else parts.push("no exit status yet");
    if (said.length > 0) parts.push(`last said:\n  ${said.slice(-20).join("\n  ")}`);
    else parts.push("said nothing");
    return ` (browser ${parts.join("; ")})`;
  };

  let port = null;
  for (let i = 0; i < 100 && port === null; i++) {
    await sleep(200);
    // The file is there before the port is written into it: an empty one
    // is a browser that has not said yet.
    try { port = Number(readFileSync(join(profile, "DevToolsActivePort"), "utf8").split("\n")[0]) || null; }
    catch {}
  }
  if (!port) cleanup(2, `${label}: browser never opened a DevTools port${browserEpitaph()}`);
  let target = null;
  for (let i = 0; i < 50 && !target; i++) {
    await sleep(200);
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      target = list.find((entry) => entry.type === "page");
    } catch {}
  }
  if (!target) cleanup(2, `${label}: no page target${browserEpitaph()}`);

  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  const b = {
    label, child, profile, nextId: 1, pending: new Map(),
    rpcResponseIds: [], promptResponseIds: [], promptRequestsSent: 0,
    contexts: new Map(), sessions: new Map(),
    // Every uncaught exception and console error from every frame, in
    // order: what a failure ships beside its screenshot.
    console: [],
    // Every native alert/confirm/prompt the page opened, in order, with the
    // words it showed. A gate that presses a control guarded by one asserts
    // on what the person was actually asked.
    dialogs: [],
    // How the next dialog is answered. Yes, because a walk that meant to
    // press a control means to get past its confirm. A walk that wants to
    // see what saying no does sets this to false for that one press: a
    // confirm is a consent, and a consent nothing ever refused is a consent
    // nobody has walked.
    acceptDialogs: true,
  };
  // From here a clean exit is answerable for what this page said.
  registry.consoles.push(b.console);
  // A CDP call that never answers is a browser that stopped answering (a
  // renderer wedged under memory pressure keeps the socket open); a driver
  // must fail then, not wait forever. No step of any journey takes a minute.
  const CALL_TIMEOUT_MS = 60_000;
  // A poll is not a step. A step of a journey may fairly take a minute, but
  // a question asked five times a second must come back in seconds or be
  // asked again: one frame busy with a big import can leave its answer
  // queued for minutes, and at a minute's patience the whole walk waits
  // behind it with nothing to show.
  //
  // This is safe because `evaluateEverywhere` answers `null` rather than
  // throwing, and its callers spin in `waitFor`: a deadline there costs one
  // more turn of the loop. The corollary is the thing to remember - a single
  // read through it, outside a wait, now reads a frame busy for longer than
  // this as "there is no such thing" rather than as a timeout. A value read
  // once and acted on belongs in a wait, or in a call of its own with the
  // full deadline.
  const POLL_TIMEOUT_MS = 4_000;
  b.sendTo = (sessionId, method, params = {}, timeoutMs = CALL_TIMEOUT_MS) => new Promise((resolve, reject) => {
    const id = b.nextId++;
    const timer = setTimeout(() => {
      if (b.pending.delete(id)) reject(new Error(`${label}: browser did not answer ${method} within ${timeoutMs / 1000}s`));
    }, timeoutMs);
    b.pending.set(id, {
      method,
      resolve: (value) => { clearTimeout(timer); resolve(value); },
      reject: (error) => { clearTimeout(timer); reject(error); },
    });
    ws.send(JSON.stringify({id, sessionId, method, params}));
  });
  b.send = (method, params = {}) => b.sendTo(undefined, method, params);
  // A browser that dies mid-journey (the machine ran out of memory under a
  // parallel build) closes this socket; every call still waiting on it must
  // fail then, or the driver sits on a promise forever and the gate never
  // reports. It did, for a quarter of an hour, before this handler.
  const abandon = (why) => {
    // The exit status and the browser's last words travel with the failure.
    // Without them this said only that the session closed, and a walk that
    // died before its first step looked like a walk with no cause.
    //
    // So does what the driver was waiting on when it went. A browser that is
    // still running when its own socket closes has said nothing useful in its
    // stderr - the dbus and UPower lines are on every green run too - and the
    // only thing left that distinguishes one such death from another is which
    // call was in flight and how the socket ended.
    const epitaph = browserEpitaph();
    // The status this epitaph could not carry. Node fires the child's `exit`
    // after the socket's close, so a death that has a status reads as having
    // none at the instant it is written. This reads it again once the event
    // loop has turned, and prints what it finds - a SIGKILL here is the
    // machine taking the browser, which is a different failure from a
    // browser that closed its own connection. `cleanup` waits for it.
    if (epitaph.includes("no exit status yet")) {
      registry.awaitingEpitaph = true;
      const closedAt = Date.now();
      // Read at the CLOSE, not at the exit. A failing driver tears the browser
      // down on its way out, so by the time the status arrives this flag is
      // set on every failure and says nothing - which is how the first real
      // occurrence (2026-09-21) came back "exited with 0 (this driver had
      // already asked it to stop)" and could not be read either way. Only a
      // stop asked for BEFORE the socket went explains the socket going.
      const askedBeforeTheClose = registry.askedToStop;
      let told = false;
      const late = () => {
        if (told) return;
        told = true;
        const status = child.signalCode
          ? `killed by ${child.signalCode}`
          : child.exitCode !== null
            ? `exited with ${child.exitCode}`
            : "still no status";
        const ours = askedBeforeTheClose
          ? " (this driver had already asked it to stop before the socket went)"
          : registry.askedToStop
            ? " (nobody had asked it to stop when the socket went; this driver asked only afterwards, on its way out)"
            : " (nobody asked it to stop)";
        console.error(
          `${label}: browser status ${Date.now() - closedAt}ms after the socket closed: ${status}${ours}`,
        );
      };
      child.once("exit", late);
      setTimeout(() => { child.off("exit", late); late(); }, 1_000).unref?.();
    }
    const waiting = [...b.pending.values()]
      .map((entry) => entry.method)
      .filter(Boolean);
    const inFlight = waiting.length > 0 ? `; waiting on ${waiting.join(", ")}` : "; waiting on nothing";
    for (const {reject} of b.pending.values()) {
      reject(new Error(`${label}: browser session ${why}${inFlight}${epitaph}`));
    }
    b.pending.clear();
  };
  // A close code tells apart a browser that hung up (1000, and whatever it put
  // in `reason`) from a connection that simply went (1006, which is what the
  // runtime reports when no close frame ever arrived).
  ws.onclose = (event) =>
    abandon(`closed with ${event && event.code ? event.code : "no code"}${event && event.reason ? ` (${event.reason})` : ""}`);
  ws.onerror = () => abandon("errored");
  ws.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.method === "Target.attachedToTarget") {
      const {sessionId, targetInfo} = message.params;
      b.sessions.set(sessionId, {target: targetInfo, contexts: new Map()});
      b.sendTo(sessionId, "Runtime.enable").catch(() => {});
      b.sendTo(sessionId, "Page.enable").catch(() => {});
    }
    if (message.method === "Target.detachedFromTarget") b.sessions.delete(message.params.sessionId);
    // A native confirm() blocks the renderer until somebody answers it, so a
    // driver that ignores one stops answering entirely - which is what the
    // agent install button does, and why nothing had ever pressed it. Every
    // dialog is recorded and accepted: a gate that cares what it said reads
    // `b.dialogs`, and no gate ever wants to sit on one forever.
    if (message.method === "Page.javascriptDialogOpening") {
      b.dialogs.push({type: message.params.type, message: message.params.message});
      const answer = {accept: b.acceptDialogs !== false};
      if (message.sessionId) b.sendTo(message.sessionId, "Page.handleJavaScriptDialog", answer).catch(() => {});
      else b.send("Page.handleJavaScriptDialog", answer).catch(() => {});
    }
    if (message.method === "Runtime.exceptionThrown") {
      const details = message.params.exceptionDetails || {};
      const description = details.exception && details.exception.description || details.text || "exception";
      b.console.push(`[${message.sessionId || "page"}] uncaught: ${description} (${details.url || ""}:${details.lineNumber || 0})`);
    }
    // Errors and warnings always; every level when a gate asks for it
    // (SWEM_CONSOLE_ALL=1), because a library that refuses quietly says why
    // in a debug line.
    const consoleLevels = process.env.SWEM_CONSOLE_ALL === "1" ? null : ["error", "warning", "assert"];
    if (message.method === "Runtime.consoleAPICalled" && (consoleLevels === null || consoleLevels.includes(message.params.type))) {
      const text = (message.params.args || []).map((arg) => arg.value !== undefined ? String(arg.value) : (arg.description || arg.type)).join(" ");
      b.console.push(`[${message.sessionId || "page"}] console.${message.params.type}: ${text}`);
    }
    if (message.method === "Runtime.executionContextCreated") {
      const bucket = message.sessionId ? b.sessions.get(message.sessionId) : null;
      (bucket ? bucket.contexts : b.contexts).set(message.params.context.id, message.params.context);
    }
    // A context that went away (a frame navigated, a worker ended) must leave
    // the map, or every later evaluate waits its full timeout on it and a
    // driver that polls the page seems to hang for a quarter of an hour.
    if (message.method === "Runtime.executionContextDestroyed") {
      const bucket = message.sessionId ? b.sessions.get(message.sessionId) : null;
      (bucket ? bucket.contexts : b.contexts).delete(message.params.executionContextId);
    }
    if (message.method === "Runtime.executionContextsCleared") {
      const bucket = message.sessionId ? b.sessions.get(message.sessionId) : null;
      (bucket ? bucket.contexts : b.contexts).clear();
    }
    if (message.method === "Network.requestWillBeSent" && message.params.request.url.endsWith("/prompt")) {
      b.promptRequestsSent += 1;
    }
    if (message.method === "Network.responseReceived") {
      const requestUrl = message.params.response.url;
      if (requestUrl.includes("/rpc")) b.rpcResponseIds.push(message.params.requestId);
      if (requestUrl.endsWith("/prompt")) b.promptResponseIds.push(message.params.requestId);
    }
    if (message.id && b.pending.has(message.id)) {
      const {resolve, reject} = b.pending.get(message.id);
      b.pending.delete(message.id);
      if (message.error) reject(new Error(message.error.message)); else resolve(message.result);
    }
  };
  await b.send("Page.enable");
  await b.send("Runtime.enable");
  await b.send("Network.enable");
  await b.send("Target.setAutoAttach", {autoAttach: true, waitForDebuggerOnStart: false, flatten: true});
  await b.send("Page.navigate", {url});

  // Evaluate in every known frame context (page and out-of-process frames);
  // the first non-empty value wins.
  // Only documents answer a page question: a worker has no document, and a
  // worker parked in `Atomics.wait` (the engine's ring readers) never answers
  // at all, so an evaluate sent there waits its whole timeout and a driver
  // polling the page seems to hang. Frames, not workers.
  const documentScopes = () => [
    [undefined, b.contexts],
    ...[...b.sessions]
      .filter(([, session]) => !/worker|worklet/i.test(session.target && session.target.type || ""))
      .map(([id, session]) => [id, session.contexts]),
  ];
  b.evaluateEverywhere = async (expression) => {
    const scopes = documentScopes();
    for (const [sessionId, contextMap] of scopes) {
      for (const [contextId] of contextMap) {
        try {
          const result = await b.sendTo(sessionId, "Runtime.evaluate", {contextId, expression, returnByValue: true}, POLL_TIMEOUT_MS);
          if (result.result && result.result.value) return result.result.value;
        } catch {}
      }
    }
    return null;
  };
  // The same expression in every context, every answer kept: what a
  // failure needs to say which frame was which.
  b.mapContexts = async (expression) => {
    const answers = [];
    const scopes = documentScopes();
    for (const [sessionId, contextMap] of scopes) {
      for (const [contextId, context] of contextMap) {
        try {
          const result = await b.sendTo(sessionId, "Runtime.evaluate", {contextId, expression, returnByValue: true});
          answers.push({session: sessionId || "page", contextId, world: context && context.auxData ? context.auxData.type : "?", value: result.result ? result.result.value : null});
        } catch (error) {
          answers.push({session: sessionId || "page", contextId, error: String(error).slice(0, 80)});
        }
      }
    }
    return answers;
  };
  b.evaluate = async (expression) => {
    const result = await b.send("Runtime.evaluate", {expression, returnByValue: true, awaitPromise: true});
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  // Hand real files to a real `<input type=file>`, wherever it lives: the
  // page, or an out-of-process App frame. A person picks files in a dialog
  // no driver can open, so this is the only way to walk that path honestly.
  b.setFileInputFiles = async (selector, files) => {
    // A frame that just re-rendered can answer with no document for a
    // moment; a person would simply try the dialog again. The input may be
    // in the page, in an out-of-process App frame, or in a same-process
    // child frame under it (a View served as a document of the sandbox
    // origin): every execution context of every session is asked.
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const scopes = [[undefined, b.contexts], ...[...b.sessions].map(([id, session]) => [id, session.contexts])];
      for (const [sessionId, contextMap] of scopes) {
        for (const [contextId, context] of contextMap) {
          // Only a frame's own world: an isolated world sees the same DOM
          // but a file set there fires no change the page's script hears.
          if (context && context.auxData && context.auxData.isDefault === false) continue;
          try {
            const found = await b.sendTo(sessionId, "Runtime.evaluate", {
              contextId, expression: `document.querySelector(${JSON.stringify(selector)})`,
            });
            if (!found.result || found.result.subtype === "null" || !found.result.objectId) continue;
            await b.sendTo(sessionId, "DOM.enable");
            await b.sendTo(sessionId, "DOM.setFileInputFiles", {objectId: found.result.objectId, files});
            // The DOM agent is on only for the call that needs a node handle.
            // Left on, it reports every mutation of the page over this socket,
            // and a surface that redraws while it plays (a waveform under a
            // moving playhead) then buries the driver's own answers: every
            // later evaluate waits its full timeout and the walk stalls with
            // nothing to show for it.
            await b.sendTo(sessionId, "DOM.disable").catch(() => {});
            // Setting the files through the protocol does not always reach
            // the page as a change; when the page has not consumed them yet
            // (a page that did resets the input), say it the way a dialog
            // would.
            const after = await b.sendTo(sessionId, "Runtime.callFunctionOn", {
              objectId: found.result.objectId,
              returnByValue: true,
              functionDeclaration: "function () { const pending = this.files && this.files.length > 0; if (pending) { this.dispatchEvent(new Event('input', {bubbles: true})); this.dispatchEvent(new Event('change', {bubbles: true})); } return {dispatched: pending, path: location.pathname}; }",
            });
            return {sessionId: sessionId || "page", contextId, ...(after.result && after.result.value ? after.result.value : {})};
          } catch {
            // Not this frame: try the next.
          }
        }
      }
      await sleep(400);
    }
    return false;
  };
  b.textOf = (id) => b.evaluate(`(document.getElementById(${JSON.stringify(id)}) || {textContent: ""}).textContent`);
  b.dataset = async (id) => JSON.parse(await b.evaluate(
    `JSON.stringify({...(document.getElementById(${JSON.stringify(id)}) || {dataset: {}}).dataset})`));
  b.exists = (selector) => b.evaluate(`document.querySelector(${JSON.stringify(selector)}) !== null`);
  // What the conversation is showing right now.
  b.bodies = () => b.evaluate(
    `JSON.stringify([...document.querySelectorAll("#conversation .message .message-body")].map(node => node.textContent))`,
  ).then(JSON.parse);
  // The same, but waited for. The turn's outcome and the messages it produced
  // reach the page on different lanes, so a read taken the moment the outcome
  // says "turn:" can find a conversation that has not been projected yet -
  // under load that is an empty list, and a walk that judged by it would call
  // the product broken for being slow. This waits for the evidence itself and
  // then hands back whatever it last saw, so the caller's own failure message
  // still says what was there.
  b.bodiesUntil = async (predicate, attempts = 100, interval = 100) => {
    let said = [];
    for (let i = 0; i < attempts; i++) {
      said = await b.bodies();
      if (predicate(said)) return said;
      await sleep(interval);
    }
    return said;
  };
  b.waitFor = async (what, predicate, attempts = 150, interval = 200) => {
    for (let i = 0; i < attempts; i++) {
      try { if (await predicate()) return; } catch {}
      await sleep(interval);
    }
    let screenshot = "no screenshot";
    try {
      const shot = await b.send("Page.captureScreenshot", {format: "png"});
      screenshot = join(failureDir, `gate-failure-${label}.png`);
      writeFileSync(screenshot, Buffer.from(shot.data, "base64"));
    } catch {}
    let consoleLog = "no console log";
    try {
      consoleLog = join(failureDir, `gate-failure-${label}.console.txt`);
      writeFileSync(consoleLog, b.console.join("\n") + "\n");
    } catch {}
    const tail = b.console.slice(-6).map((line) => `\n  ${line.slice(0, 400)}`).join("");
    // A walk that timed out having also raced its handshake has two stories to
    // tell, and the reader should not have to find the second one in the tail.
    const jumped = handshakeViolations(b.console).length;
    const handshake = jumped === 0 ? "" : `; handshake jumped ${jumped}x`;
    const message = `${label}: timed out waiting for ${what}; apps=${await b.textOf("apps-status")}; turn=${await b.textOf("turn-outcome")}${handshake}; screenshot=${screenshot}; console=${consoleLog}${tail}`;
    cleanup(2, message);
    // `cleanup` defers the exit by 300ms so the browser can be killed first.
    // Without this throw the driver keeps running in that window and prints
    // the steps after the failed one, which reads as "it got further than it
    // did" and sends the reader looking in the wrong place.
    throw new Error(message);
  };
  /// The surface as a person sees it, written only when a reader asked for
  /// one with SWEM_SCREENSHOT_DIR. Every walk can be looked at this way,
  /// because a walk that passes can still be drawing something wrong, and
  /// patching a driver by hand to take one look has twice destroyed a probe
  /// that was in the file at the time.
  ///
  /// 1600 by 1000 is an ordinary laptop. The walk's own window is wider, so a
  /// layout fault that only shows at a real width is invisible without this.
  b.shoot = async (name) => {
    const dir = process.env.SWEM_SCREENSHOT_DIR;
    if (!dir) return null;
    await b.send("Emulation.setDeviceMetricsOverride", {width: 1600, height: 1000, deviceScaleFactor: 1, mobile: false});
    await sleep(600);
    const shot = await b.send("Page.captureScreenshot", {format: "png"});
    const path = join(dir, `${label}-${name}.png`);
    writeFileSync(path, Buffer.from(shot.data, "base64"));
    await b.send("Emulation.clearDeviceMetricsOverride");
    await sleep(200);
    return path;
  };
  b.clickAt = async (x, y) => {
    await b.send("Input.dispatchMouseEvent", {type: "mouseMoved", x, y, pointerType: "mouse"});
    await b.send("Input.dispatchMouseEvent", {type: "mousePressed", x, y, button: "left", buttons: 1, clickCount: 1, pointerType: "mouse"});
    await b.send("Input.dispatchMouseEvent", {type: "mouseReleased", x, y, button: "left", clickCount: 1, pointerType: "mouse"});
  };
  // A real click at the element's centre, scrolled into view first.
  b.click = async (selector) => {
    // A control below the fold of a scrolling rail has a viewport point
    // outside the window, and a click there lands nowhere at all.
    await b.evaluate(`document.querySelector(${JSON.stringify(selector)}).scrollIntoView({block: "center", inline: "nearest"})`);
    await sleep(100);
    const rect = JSON.parse(await b.evaluate(
      `JSON.stringify(document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect())`));
    await b.clickAt(rect.left + rect.width / 2, rect.top + rect.height / 2);
  };
  b.responseBody = async (requestId) => {
    const response = await b.send("Network.getResponseBody", {requestId});
    return JSON.parse(response.body);
  };
  /// Fails the walk when any View called the host before its handshake
  /// finished. Free to call: the page console is collected anyway, and
  /// `console.warn` is a `warning`, which this helper keeps by default.
  b.assertHandshakeOrder = (where = label) => assertHandshakeOrder(b.console, where, (message) => {
    cleanup(2, message);
    throw new Error(message);
  });
  // ---- what a person does in the page, in their words -----------------
  //
  // A driver goes where a person goes and does what a person does: it goes
  // to a place by its address, fills in what the page asks for, says
  // something in a chat and reads what was said. What happened is read
  // from the product's own record, through the page's own door.

  /// Ask the product something from the page, as the page does: beside
  /// where the page was opened, which is the root of its address or, built
  /// into a product, a path of that product's server.
  b.ask = async (path, init) => JSON.parse(await b.evaluate(`(async () => {
    const response = await fetch(new URL(${JSON.stringify(path)}.replace(/^\\//, ""), document.baseURI), ${JSON.stringify(init ?? {})});
    return JSON.stringify({status: response.status, body: await response.json().catch(() => null)});
  })()`));
  b.goTo = async (hash) => {
    await b.evaluate(`location.hash = ${JSON.stringify(hash)}`);
    await sleep(400);
  };
  /// Type into a field the page draws, the way typing does: the value and
  /// the event React listens for.
  b.fill = async (selector, value) => b.evaluate(`(() => {
    const field = document.querySelector(${JSON.stringify(selector)});
    if (!field) return false;
    const kind = field instanceof HTMLTextAreaElement ? HTMLTextAreaElement : HTMLInputElement;
    Object.getOwnPropertyDescriptor(kind.prototype, "value").set.call(field, ${JSON.stringify(value)});
    field.dispatchEvent(new Event("input", {bubbles: true}));
    return true;
  })()`);
  /// Press whatever says these words, among what matches the selector.
  b.pressText = async (selector, text) => {
    const at = JSON.parse(await b.evaluate(`(() => {
      const found = [...document.querySelectorAll(${JSON.stringify(selector)})].find((one) => one.innerText.includes(${JSON.stringify(text)}));
      if (!found) return "null";
      found.scrollIntoView({block: "center"});
      const box = found.getBoundingClientRect();
      return JSON.stringify({x: box.left + box.width / 2, y: box.top + box.height / 2});
    })()`));
    if (!at) return false;
    await b.clickAt(at.x, at.y);
    await sleep(300);
    return true;
  };
  /// Answer what the page asks before it installs something, and hand
  /// back what it said: a consent is to words, and a walk reads them.
  b.consent = async (agree = true) => {
    await b.waitFor("the consent question", async () => b.exists('[role="dialog"]'), 200);
    const asked = await b.evaluate(`document.querySelector('[role="dialog"]')?.innerText ?? ""`);
    // Yes is the dialog's one primary button, whatever it says: Install,
    // Update, Remove.
    if (agree) {
      if (!(await b.exists('[role="dialog"] .k-btn.k-primary'))) cleanup(1, `the question has no way to say yes: ${JSON.stringify(asked)}`);
      await b.click('[role="dialog"] .k-btn.k-primary');
    } else if (!(await b.pressText('[role="dialog"] button', "Not now"))) {
      cleanup(1, `the question has no way to answer it: ${JSON.stringify(asked)}`);
    }
    await b.waitFor("the question is answered", async () => !(await b.exists('[role="dialog"]')), 100);
    return asked;
  };
  /// The participant an agent is, by the name it was given.
  b.agentOf = async (profile) => {
    const people = (await b.ask("/api/people")).body;
    return (people?.participants ?? []).find((one) => one.profile_id === profile)?.participant_id ?? null;
  };
  /// Make an agent: a name, and the engine it stands on.
  b.makeAgent = async (engine, name = engine) => {
    await b.goTo("#/agents/new");
    await b.waitFor("the new-agent form", async () => b.exists(".w-form input.k-field"));
    const offered = (await b.ask("/api/onboarding")).body;
    const called = (offered?.agents ?? []).find((one) => one.agent_id === engine)?.name;
    if (!called) cleanup(1, `this computer offers no engine ${engine}: ${JSON.stringify(offered)}`);
    await b.fill(".w-form input.k-field", name);
    if (!(await b.pressText(".w-form label", called))) cleanup(1, `the form does not offer ${called}`);
    // Four steps; a person in a hurry leaves after the second.
    const enabled = (says) => b.evaluate(`[...document.querySelectorAll(".w-form button")].some((one) => one.innerText.trim() === ${JSON.stringify(says)} && !one.disabled)`);
    await b.waitFor("the first step is done", async () => enabled("Continue"), 100);
    if (!(await b.pressText(".w-form button", "Continue"))) cleanup(1, "the first step has no way on");
    await b.waitFor("the second step", async () => enabled("Skip the rest and create"), 100);
    if (!(await b.pressText(".w-form button", "Skip the rest and create"))) cleanup(1, "the second step has no way to make the agent");
    await b.waitFor("the agent is made", async () => (await b.agentOf(name)) !== null, 200);
    await b.waitFor("the page goes to it", async () => b.evaluate(`location.hash.startsWith("#/agents/p_")`), 100);
    return name;
  };
  /// Go to an agent: its chat, its files, its terminal or its settings.
  b.openAgent = async (profile, tab = "chat") => {
    const agent = await b.agentOf(profile);
    if (!agent) cleanup(1, `there is no agent ${profile}: ${JSON.stringify(await b.ask("/api/people"))}`);
    await b.goTo(`#/agents/${agent}${tab === "chat" ? "" : `/${tab}`}`);
    await b.waitFor(`the agent's ${tab}`, async () => b.exists(`nav[aria-label] .k-tab.k-active`), 100);
    return agent;
  };
  /// The chat the page is showing, read from the product's record: the one
  /// its address names, or the agent's latest chat when it names the agent.
  b.chat = async () => {
    const hash = await b.evaluate("location.hash");
    let shown = hash.split("/chats/")[1] ?? "";
    if (!shown && hash.startsWith("#/agents/")) {
      const agent = decodeURIComponent(hash.split("/")[2] ?? "");
      const chats = (await b.ask("/api/chats")).body ?? [];
      const alone = (chat) => chat.members.filter((member) => member.kind === "agent");
      shown = chats.find((chat) => alone(chat).length === 1 && alone(chat)[0].participant_id === agent)?.chat_id ?? "";
    }
    if (!shown || shown === "new") return null;
    return (await b.ask(`/api/chats/${encodeURIComponent(shown)}?limit=2000`)).body;
  };
  /// What was said in the chat the page is showing, oldest first.
  b.said = async () => ((await b.chat())?.messages ?? []).map((message) => message.text);
  /// Say something in the chat the page is showing and wait until it was
  /// answered. Returns everything said in the chat.
  b.say = async (text, {files = [], answered = true} = {}) => {
    const before = (await b.said()).length;
    await b.waitFor("the composer", async () => b.exists(".w-composer-input"));
    if (files.length > 0 && !(await b.setFileInputFiles('.w-composer input[type="file"]', files))) {
      cleanup(2, "the composer takes no files");
    }
    await b.fill(".w-composer-input", text);
    await b.waitFor("Send", async () => b.evaluate(`!document.querySelector('.w-composer button[type="submit"]')?.disabled`));
    await b.click('.w-composer button[type="submit"]');
    if (!answered) return b.said();
    await b.waitFor("the answer", async () => {
      const chat = await b.chat();
      return chat !== null && chat.deliveries.length === 0 && chat.messages.length >= before + 2;
    }, 400);
    return b.said();
  };
  /// Put an agent to sleep from its header, once the page offers it: the
  /// page learns that a turn ended a moment after the record does.
  b.putToSleep = async (chat, agent) => {
    await b.waitFor("the agent free to be put to sleep", async () => b.evaluate(`(() => {
      const found = [...document.querySelectorAll(".w-head button")].find((one) => one.innerText.includes("Put to sleep"));
      return Boolean(found) && !found.disabled;
    })()`), 300);
    await b.pressText(".w-head button", "Put to sleep");
    await b.waitFor("the agent sleeps", async () =>
      (await b.ask(`/api/chats/${chat}/agents/${agent}/session`)).body?.connection_id === null, 300);
  };
  b.close = async () => {
    try { ws.close(); } catch {}
    try { child.kill(); } catch {}
    await sleep(500);
    try { rmSync(profile, {recursive: true, force: true}); } catch {}
  };
  // A device that holds passkeys, in the browser itself: what a person's
  // fingerprint answers is answered by it.
  b.holdPasskeys = async () => {
    await b.send("WebAuthn.enable", {});
    const made = await b.send("WebAuthn.addVirtualAuthenticator", {options: {
      protocol: "ctap2", transport: "internal", hasResidentKey: true, hasUserVerification: true,
      isUserVerified: true, automaticPresenceSimulation: true,
    }});
    return made.authenticatorId;
  };
  b.forgetPasskeys = async (authenticatorId) => b.send("WebAuthn.removeVirtualAuthenticator", {authenticatorId});
  await b.waitFor("production Workbench renderer", async () => b.evaluate(`
    window.__SWEM_WORKBENCH_WEB__ === "react-19" &&
    document.getElementById("workbench-root")?.dataset.renderer === "react" &&
    document.querySelector(${JSON.stringify(door ? "nav[aria-label=\"Workbench\"], .w-alone" : "nav[aria-label=\"Workbench\"]")}) !== null
  `));
  return b;
}
