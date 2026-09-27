// The Apps the servers of an agent bring, in its chat: the ones a person
// opens, and the one a tool of the agent brought while it worked.
//
// An App is mounted by the Apps bridge into a container React never draws
// again, so its frame keeps its identity while the chat around it changes.

import { useEffect, useRef, useState } from "react";

import { loadBridge, type MountedApp } from "../agent/bridge.ts";
import type { AppAttachment, OpenedApp } from "../agent/session.ts";
import { fetchJson } from "../http.ts";
import { Cross, Tiles } from "./icons.tsx";
import { sessionOf } from "./session.ts";

const part = encodeURIComponent;
const post = (body: unknown): RequestInit => ({
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify(body),
});

/// What an App is called when its server gives it no title: the last part
/// of its address, as words.
const named = (app: { uri: string; title?: string; name?: string }): string =>
  app.title ?? app.name ?? (app.uri.split("/").pop() ?? app.uri).replace(/[-_]/g, " ");

interface Observed {
  opened: OpenedApp;
  observation: { cursor: number; status?: string; observation_id?: string };
}

export function ChatApps({
  chat,
  agent,
  name,
  brought,
  shown,
  onShow,
  onHide,
}: {
  chat: string;
  agent: string;
  name: string;
  /// The place at which a tool of the agent last brought an App.
  brought: number;
  shown: boolean;
  onShow: () => void;
  onHide: () => void;
}) {
  const container = useRef<HTMLDivElement>(null);
  const mounted = useRef<{ app: MountedApp; opened: OpenedApp; connection: string } | null>(null);
  const seen = useRef(0);
  const [attachments, setAttachments] = useState<AppAttachment[] | null>(null);
  const [status, setStatus] = useState("");
  const [open, setOpen] = useState<OpenedApp | null>(null);

  const close = async () => {
    const closing = mounted.current;
    mounted.current = null;
    setOpen(null);
    if (!closing) return;
    await closing.app.teardown().catch(() => {});
    await fetchJson(`/api/connections/${part(closing.connection)}/apps/${part(closing.opened.app_id)}/close`, post({})).catch(() => {});
  };

  const mount = async (connection: string, opened: OpenedApp, observation: Observed["observation"] | null) => {
    await close();
    if (!(await loadBridge()) || !window.SwemAppsBridge || !container.current) {
      setStatus("Apps are off in this build; what they do stays available to the agent as tools.");
      return;
    }
    setOpen(opened);
    const relay = (message: unknown) => fetchJson<unknown>(`/api/connections/${part(connection)}/apps/${part(opened.app_id)}/rpc`, post(message));
    const app = await window.SwemAppsBridge.mount({ container: container.current, opened, relay, observation, onStatus: setStatus });
    mounted.current = { app, opened, connection };
    if (observation?.status === "pending" && observation.observation_id) {
      const terminal = await fetchJson<unknown>(
        `/api/connections/${part(connection)}/apps/observations/${part(observation.observation_id)}?wait_ms=25000`,
      ).catch(() => null);
      if (terminal && mounted.current?.app === app) await app.deliver?.(terminal);
    }
  };

  // What the person asked to see: the engine is started for it, because
  // what its servers bring is known only to a session.
  useEffect(() => {
    if (!shown || attachments !== null) return undefined;
    let left = false;
    setStatus("Asking what its servers bring…");
    void (async () => {
      try {
        const connection = await sessionOf(chat, agent, true);
        if (!connection || left) return;
        const found = await fetchJson<AppAttachment[]>(`/api/connections/${part(connection)}/apps`);
        if (left) return;
        setAttachments(found);
        setStatus("");
      } catch (error) {
        if (!left) setStatus((error as Error).message);
      }
    })();
    return () => {
      left = true;
    };
  }, [shown, attachments, chat, agent]);

  // What a tool of the agent brought: it is shown as it arrives.
  useEffect(() => {
    if (!brought || brought <= seen.current) return;
    void (async () => {
      try {
        const connection = await sessionOf(chat, agent, false);
        if (!connection) return;
        const observed = await fetchJson<Observed | null>(
          `/api/connections/${part(connection)}/apps/observations/next?after=${seen.current}&wait_ms=2000`,
        );
        if (!observed) return;
        seen.current = Math.max(brought, observed.observation.cursor);
        onShow();
        await mount(connection, observed.opened, observed.observation);
      } catch (error) {
        setStatus((error as Error).message);
      }
    })();
    // `mount` and `onShow` are this component's own and change with every draw.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [brought, chat, agent]);

  useEffect(() => () => void close(), []);

  const openApp = async (server: string, uri: string) => {
    try {
      const connection = await sessionOf(chat, agent, true);
      if (!connection) return;
      const opened = await fetchJson<OpenedApp>(`/api/connections/${part(connection)}/apps/open`, post({ server_name: server, uri }));
      await mount(connection, opened, null);
    } catch (error) {
      setStatus(`It could not be opened: ${(error as Error).message}`);
    }
  };

  const apps = (attachments ?? []).flatMap((attachment) => attachment.apps.map((app) => ({ server: attachment.server_name, app })));
  return (
    <aside className={`w-side w-right w-apps${open ? " w-wide" : ""}`} aria-label={`Apps of ${name}`} hidden={!shown}>
      <div className="k-spread">
        <span className="k-eyebrow">Apps of {name}</span>
        <button type="button" className="k-btn k-quiet" aria-label="Hide the Apps" onClick={onHide}>
          <Cross size={14} />
        </button>
      </div>
      {open ? (
        <div className="k-spread">
          <span className="k-name w-one-line">{named(open)}</span>
          <button type="button" className="k-btn" onClick={() => void close()}>
            Close it
          </button>
        </div>
      ) : (
        <div className="k-rail-group">
          {apps.map(({ server, app }) => (
            <button type="button" className="k-rail-item" key={`${server} ${app.uri}`} title={app.uri} onClick={() => void openApp(server, app.uri)}>
              <Tiles />
              <span className="k-two">
                <span className="k-name">{named(app)}</span>
                <span className="k-caption">{server}</span>
              </span>
            </button>
          ))}
          {attachments !== null && apps.length === 0 ? <span className="k-caption">The servers it attaches bring no Apps.</span> : null}
        </div>
      )}
      {status ? <span className="k-caption">{status}</span> : null}
      <div className="w-app" ref={container} />
    </aside>
  );
}
