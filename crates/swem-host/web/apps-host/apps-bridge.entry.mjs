// Host-side bundle entry for the SWEM generic Apps host.
//
// The OFFICIAL MCP Apps AppBridge (SEP-1865, @modelcontextprotocol/ext-apps)
// speaks the postMessage wire to the sandbox proxy; every request an App
// makes is forwarded to the Rust relay endpoint, where the ONLY security
// decisions live (method allowlist, tool visibility, cross-server isolation).
// This bundle is transport, never policy. JSON-RPC ids are integers - a float
// id parses as a notification (the 0.25 lesson).
//
// Built at test/serve time with `npm ci && npx esbuild --bundle`; the
// committed package-lock.json is the provenance, the bundle is never
// committed.

import { AppBridge } from "@modelcontextprotocol/ext-apps/app-bridge";
import { PostMessageTransport } from "@modelcontextprotocol/ext-apps/app-bridge";
import { readHostContext, watchHostContext } from './theme.mjs';

let nextRelayId = 1;

async function relayRequest(relay, method, params) {
  const response = await relay({
    jsonrpc: "2.0",
    id: nextRelayId++,
    method,
    params,
  });
  if (!response || response.error) {
    const message = response && response.error ? response.error.message : "relay failed";
    throw new Error(message);
  }
  return response.result;
}

/**
 * Mount one opened App into `container`.
 *
 * options:
 *  - container: HTMLElement receiving the sandbox iframe
 *  - opened: the shell's open-app payload
 *    {app_id, server_name, uri, html, csp, permissions, sandbox_url, sandbox_origin}
 *  - relay: async (jsonRpcMessage) => jsonRpcResponse  (POSTs to the Rust relay)
 *  - fit: "content" (frame height follows the App) | "fill" (the container's)
 *  - onStatus: (text) => void
 *
 * Returns { teardown: async () => void, bridge }.
 */
// MCP Apps permission keys -> Permissions Policy features, granted to the
// exact sandbox origin on the outer frame. Unknown keys are ignored.
const PERMISSION_FEATURES = {
  camera: "camera",
  microphone: "microphone",
  geolocation: "geolocation",
  clipboardWrite: "clipboard-write",
  midi: "midi",
};
function permissionsToAllow(permissions, origin) {
  if (!permissions || typeof permissions !== "object" || !origin) return "";
  return Object.keys(permissions)
    .map((key) => PERMISSION_FEATURES[key])
    .filter(Boolean)
    .map((feature) => `${feature} ${origin}`)
    .join("; ");
}

export async function mount({ container, opened, relay, observation, onStatus, signal, fit = "content" }) {
  signal?.throwIfAborted();
  const status = onStatus || (() => {});
  if (!opened.sandbox_url || !opened.sandbox_origin) {
    throw new Error("the shell exposes no sandbox origin; Apps are disabled");
  }
  const iframe = document.createElement("iframe");
  iframe.id = "app-frame";
  // Safe because the sandbox document lives on a DIFFERENT origin (second
  // 127.0.0.1 port): same-origin only within the sandbox, never to the shell.
  iframe.setAttribute("sandbox", "allow-scripts allow-same-origin allow-forms");
  // Permissions Policy delegation is per frame: the App's declared
  // permissions must be allowed on this outer frame (for the sandbox
  // origin) before the proxy can grant them to the View.
  let allow = permissionsToAllow(opened.permissions, opened.sandbox_origin);
  // Cross-origin isolation is a Permissions Policy feature that defaults to
  // the top-level origin: a View on the sandbox origin is only isolated
  // (SharedArrayBuffer) when the shell delegates it to that origin.
  if (opened.isolated && opened.sandbox_origin) {
    allow = `${allow ? `${allow}; ` : ""}cross-origin-isolated ${opened.sandbox_origin}`;
  }
  if (allow) iframe.setAttribute("allow", allow);
  // The frame takes the width its container gives it and the height the
  // App asks for (`ui/notifications/size-changed`); until the App speaks it
  // has a modest default, never a hidden fold of the App's own page.
  iframe.style.width = "100%";
  iframe.style.height = "360px";
  iframe.style.border = opened.prefers_border ? "1px solid #888" : "none";
  // The Rust-resolved CSP travels as a query parameter and comes back as a
  // REAL HTTP header on the sandbox document; the srcdoc inner iframe
  // inherits it. The header is host authority, not iframe self-description.
  // The proxy validates every host message against this exact origin.
  // The connection and App handles let the View address its server's script
  // resources on the sandbox origin (`document.baseURI` of the srcdoc View
  // is this URL): /apps/{connection}/{app}/resources?uri=ui://...
  iframe.src = opened.sandbox_url +
    "?csp=" + encodeURIComponent(opened.csp) +
    "&host=" + encodeURIComponent(window.location.origin) +
    "&connection=" + encodeURIComponent(opened.connection_id || "") +
    "&app=" + encodeURIComponent(opened.app_id || "");
  container.appendChild(iframe);

  const bridge = new AppBridge(
    null,
    { name: "swem-workbench-shell", version: "0.1" },
    {},
    { hostContext: readHostContext() },
  );
  // `fit: "content"` (a surface in a rail): the frame grows to the height
  // the App announces. `fit: "fill"` (a workbench under a rail): the
  // container owns the area and the App lays itself out inside it.
  if (fit === "content") {
    bridge.onsizechange = (size) => {
      const height = Number(size && size.height);
      if (Number.isFinite(height) && height > 0) iframe.style.height = `${Math.min(Math.ceil(height), 6000)}px`;
      // Wider than its rail when the App needs it: the rail scrolls.
      const width = Number(size && size.width);
      const room = container.clientWidth || 0;
      if (Number.isFinite(width) && width > room + 1) iframe.style.width = `${Math.min(Math.ceil(width), 4000)}px`;
      else iframe.style.width = "100%";
    };
  }
  // A View that calls before its handshake is refused, not served.
  //
  // The vendored bridge warns about such a call and then runs the handler
  // anyway, so until now the host answered it - and whether the answer was
  // right depended on whether the host had already published what was asked
  // for. That makes the fault invisible exactly while it is harmless and
  // invisible again when it is not. It cost a real morning: fourteen of the
  // thirty-three product walks jumped the handshake on every run and every
  // one of them passed, and a fix for it lived an hour and passed its own
  // single run while still being wrong.
  //
  // So the rule is enforced where it is broken, not only where it is
  // measured. The App gets an error naming the rule and can say so in its
  // own status line, which is what a person can act on; a walk still goes
  // red on the warning, which is what we can act on.
  let handshakeDone = false;
  const afterHandshake = (method, call) => {
    if (!handshakeDone) {
      // Said out loud as well as answered. The App turns this into its own
      // status line, which is what a person needs; but a refusal that leaves
      // no trace on the page is invisible to whoever is debugging, and it
      // would leave the browser walks depending on the vendored bridge's
      // warning - one upgrade away from a check that quietly passes forever.
      console.error(`[swem-apps] refused a pre-handshake ${method} from the View`);
      return Promise.reject(new Error(
        `${method} refused: this View called the host before completing the ui/initialize handshake. ` +
        "Await app.connect() before the first host call.",
      ));
    }
    return call();
  };
  bridge.oncalltool = (params) => afterHandshake("tools/call", () => relayRequest(relay, "tools/call", params));
  bridge.onreadresource = (params) => afterHandshake("resources/read", () => relayRequest(relay, "resources/read", params));
  const initialized = new Promise((resolve) => {
    bridge.oninitialized = () => {
      handshakeDone = true;
      status("app initialized");
      resolve();
    };
  });
  bridge.onsandboxready = async () => {
    status("sandbox ready");
    // An App that asked for a real origin gets its View as a document of
    // the sandbox origin (storage, workers, isolation); the others stay an
    // opaque srcdoc document, as the spec renders them.
    await bridge.sendSandboxResourceReady(opened.isolated && opened.view_url ? {
      html: "",
      url: opened.view_url,
      sandbox: "allow-scripts allow-same-origin allow-forms",
      permissions: opened.permissions || {},
    } : {
      html: opened.html,
      sandbox: "allow-scripts",
      permissions: opened.permissions || {},
    });
  };

  const transport = new PostMessageTransport(iframe.contentWindow, iframe.contentWindow);
  let abortOpening;
  let openingTimer;
  const interrupted = new Promise((_, reject) => {
    abortOpening = () => reject(new Error("App opening cancelled"));
    signal?.addEventListener("abort", abortOpening, { once: true });
    openingTimer = setTimeout(() => reject(new Error("App initialization timed out")), 30000);
  });
  try {
    await Promise.race([bridge.connect(transport).then(() => {
      status("bridge connected");
      return initialized;
    }), interrupted]);
    signal?.throwIfAborted();
  } catch (error) {
    try { await bridge.close(); }
    finally { iframe.remove(); }
    throw error;
  } finally {
    clearTimeout(openingTimer);
    signal?.removeEventListener("abort", abortOpening);
  }
  const stopWatchingTheme = watchHostContext(bridge);
  bridge.setHostContext(readHostContext());
  let inputSent = false;
  let terminalSent = false;
  const deliver = async (call) => {
    if (!inputSent) {
      await bridge.sendToolInput({ arguments: call ? call.arguments : {} });
      inputSent = true;
    }
    if (call && ["completed", "tool_error"].includes(call.status) &&
        call.result && !terminalSent) {
      await bridge.sendToolResult(call.result);
      terminalSent = true;
      status(call.status === "tool_error"
        ? "agent App tool error delivered"
        : "agent App result delivered");
    } else if (call && call.status === "cancelled" && !terminalSent) {
      await bridge.sendToolCancelled({
        reason: call.cancellation_reason || undefined,
      });
      terminalSent = true;
      status("agent App cancellation delivered");
    } else if (call && call.status === "protocol_error") {
      // MCP Apps has no Host -> View notification for a JSON-RPC protocol
      // error. Do not fabricate either a CallToolResult or a cancellation.
      status("agent App protocol error observed");
    }
  };
  // User-opened Apps receive the required empty input. Agent-initiated Apps
  // receive exact observed arguments and, when already complete, the original
  // CallToolResult in the same normative input-before-result order.
  status("app ready");
  await deliver(observation || null);

  return {
    bridge,
    deliver,
    teardown: async () => {
      stopWatchingTheme();
      try {
        await bridge.teardownResource({}, { timeout: 2000 });
      } finally {
        try { await bridge.close(); }
        finally { iframe.remove(); }
      }
    },
  };
}

window.SwemAppsBridge = { mount };
