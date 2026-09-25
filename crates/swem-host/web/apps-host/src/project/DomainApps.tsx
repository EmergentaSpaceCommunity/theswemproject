import { useEffect, useRef, useState } from "react";
import { fetchJson } from "./api";
import type { AppView, ToolView } from "./types";

interface Attachment { apps: AppView[]; tools: ToolView[] }
export interface Published { apps: AppView[]; tools: ToolView[] }
interface Opened { app_id: string }
/// `mount` has always returned the AppBridge; this interface used to name only
/// `teardown` and so threw it away, which is why a domain workbench had no way
/// to hear that anything had changed and offered a Refresh button instead.
interface Mounted {
  teardown(): Promise<void>;
  bridge: { sendResourceListChanged(): Promise<void> };
}
interface Bridge {
  mount(options: {container: HTMLElement; opened: Opened; signal: AbortSignal; relay(message: unknown): Promise<unknown>; onStatus(message: string): void; fit?: "content" | "fill"}): Promise<Mounted>;
}
const post = <T,>(path: string, body: unknown) => fetchJson<T>(path, {
  method: "POST", headers: {"content-type":"application/json"}, body: JSON.stringify(body),
});

const NOTHING: Published = { apps: [], tools: [] };

/// The domain workbenches a project publishes, for the tab strip that opens
/// them, and the tools its server declares, for the forms a rung offers.
/// The server owns the App and its vocabulary; no profile or native agent
/// connection takes part in this entry point.
export function useDomainApps(server: string | null): Published {
  const [published, setPublished] = useState<Published>(NOTHING);
  useEffect(() => {
    let disposed = false;
    setPublished(NOTHING);
    if (server) {
      fetchJson<Attachment[]>(`/api/projects/${encodeURIComponent(server)}/apps`)
        .then(rows => {
          if (!disposed) setPublished({ apps: rows.flatMap(row => row.apps), tools: rows.flatMap(row => row.tools ?? []) });
        })
        .catch(() => { if (!disposed) setPublished(NOTHING); });
    }
    return () => { disposed = true; };
  }, [server]);
  return published;
}

/// One mounted domain workbench, filling the surface it was given. It is the
/// main area rather than a panel under the record browser: the timeline, not
/// the digests around it, is what a person came to work in.
export function DomainApp({server, uri, slot, changed}: {server: string; uri: string; slot?: string; changed: string | null}) {
  const container = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState("");
  const live = useRef<Mounted | null>(null);
  // Whatever changed the project - this person, their agent, or a stock MCP
  // client over the same journal - the mounted App is told that the server's
  // resources moved and re-reads the view it is a projection of. The standard
  // carries no per-URI host->view notification, so this is the list-changed
  // one; the App reads `swem://project` back through the relay, which the
  // method allowlist already permits.
  useEffect(() => {
    if (!changed) return;
    void live.current?.bridge.sendResourceListChanged().catch(() => {});
  }, [changed]);
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
        if (opened) await post(`/api/project-apps/${opened.app_id}/close`, {});
      }
    };
    void (async () => {
      try {
        setStatus("Opening…");
        await host.SwemLoadAppsBridge?.();
        const bridge = host.SwemAppsBridge;
        if (!bridge) throw new Error("the Apps bridge did not load");
        if (disposed) { await close(); return; }
        // Two slots of the same kind publish the same `ui://` resource, so
        // the App learns which line of work it was opened for from the open
        // itself, and not from the resource, which cannot know.
        opened = await post<Opened>(`/api/projects/${encodeURIComponent(server)}/apps/open`, {uri, slot});
        if (disposed) { await close(); return; }
        mounted = await bridge.mount({container: target, opened, signal: opening.signal, fit: "fill",
          relay: message => post(`/api/project-apps/${opened!.app_id}/rpc`, message),
          // The steps of opening are worth a line; a ready App is not.
          onStatus: message => { if (!disposed) setStatus(message === "app ready" ? "" : message); },
        });
        if (disposed) { await close(); return; }
        live.current = mounted;
        // The App owns the whole area it was given; its own page decides how
        // to lay the surface out inside it.
        const frame = target.querySelector("iframe");
        if (frame) { frame.style.width = "100%"; frame.style.height = "100%"; frame.style.border = "0"; }
      } catch (error) { if (!disposed) setStatus(String(error)); await close().catch(() => {}); }
    })();
    return () => {
      disposed = true; live.current = null; target.hidden = true; opening.abort();
      if (mounted) void close().catch(() => {});
    };
  }, [server, uri, slot]);
  return (
    <section className="domain-app" aria-label="Domain workbench">
      {status ? <p className="app-status" role="status">{status}</p> : null}
      <div className="app-host" ref={container} />
    </section>
  );
}
