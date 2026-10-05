import { useEffect, useRef, useState } from "react";

import { fetchJson } from "../http.ts";
import { sessionStore } from "../agent/store.ts";

interface Opened { app_id: string }
interface Mounted {
  teardown(): Promise<void>;
}
/// What an App may say the model should know (`ui/update-model-context`):
/// content blocks, each update replacing the one before.
export interface ModelContextUpdate {
  content?: unknown[];
  structuredContent?: Record<string, unknown>;
}
interface Bridge {
  mount(options: {
    container: HTMLElement;
    opened: Opened;
    signal: AbortSignal;
    relay(message: unknown): Promise<unknown>;
    onStatus(message: string): void;
    onModelContext?(update: ModelContextUpdate): Promise<void>;
    fit?: "content" | "fill";
    displayMode?: "inline" | "fullscreen";
  }): Promise<Mounted>;
}
const post = <T,>(path: string, body: unknown) => fetchJson<T>(path, {
  method: "POST", headers: {"content-type":"application/json"}, body: JSON.stringify(body),
});

/// The home App of one server, filling the space it was given. The server
/// owns the App and everything it shows; the host opens it, relays its
/// calls to that server and nothing else, and hears what it says a person
/// is looking at. No agent session takes part in any of it.
export function ServerApp({server, uri}: {server: string; uri: string}) {
  const container = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState("");
  useEffect(() => {
    if (!container.current) return undefined;
    const host = window as unknown as {SwemAppsBridge?: Bridge; SwemLoadAppsBridge?: () => Promise<boolean>};
    let disposed = false, opened: Opened | undefined, mounted: Mounted | undefined;
    const opening = new AbortController();
    const target = document.createElement("div");
    target.className = "app-mount";
    container.current.appendChild(target);
    const close = async () => {
      target.hidden = true;
      try { await mounted?.teardown(); }
      finally {
        target.remove();
        if (opened) await post(`/api/space-apps/${opened.app_id}/close`, {});
      }
    };
    void (async () => {
      try {
        setStatus("Opening…");
        await host.SwemLoadAppsBridge?.();
        const bridge = host.SwemAppsBridge;
        if (!bridge) throw new Error("the Apps bridge did not load");
        if (disposed) { await close(); return; }
        opened = await post<Opened>(`/api/spaces/${encodeURIComponent(server)}/open`, {uri});
        if (disposed) { await close(); return; }
        mounted = await bridge.mount({container: target, opened, signal: opening.signal, fit: "fill", displayMode: "fullscreen",
          relay: message => post(`/api/space-apps/${opened!.app_id}/rpc`, message),
          // The steps of opening are worth a line; a ready App is not.
          onStatus: message => { if (!disposed) setStatus(message === "app ready" ? "" : message); },
          onModelContext: update => sessionStore.offerContext(server, update),
        });
        if (disposed) { await close(); return; }
        // The App owns the whole area it was given; its own page decides how
        // to lay the surface out inside it.
        const frame = target.querySelector("iframe");
        if (frame) { frame.style.width = "100%"; frame.style.height = "100%"; frame.style.border = "0"; }
      } catch (error) { if (!disposed) setStatus(String(error)); await close().catch(() => {}); }
    })();
    return () => {
      disposed = true; target.hidden = true; opening.abort();
      if (mounted) void close().catch(() => {});
    };
  }, [server, uri]);
  return (
    <section className="domain-app" aria-label={`The App of ${server}`}>
      {status ? <p className="app-status" role="status">{status}</p> : null}
      <div className="app-host" ref={container} />
    </section>
  );
}
