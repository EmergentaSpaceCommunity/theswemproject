// The Apps the servers of an agent bring, in its chat: the ones a person
// opens, and the one a tool of the agent brought while it worked.
//
// An App is mounted by the Apps bridge into a container React never draws
// again, so its frame keeps its identity while the chat around it changes.

import { useEffect, useRef, useState } from "react";

import { loadBridge, type MountedApp } from "../agent/bridge.ts";
import { fits, type AppAttachment, type AppTool, type OpenedApp, type Platform } from "../agent/session.ts";
import { fetchJson } from "../http.ts";
import { AppForm, forAPerson, toolNamed } from "./AppForm.tsx";
import { Cross, Tiles } from "./icons.tsx";
import { sessionOf } from "./session.ts";
import type { Brought } from "./timeline.ts";

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

/// Per chat and agent, the place of the last App shown because a tool
/// brought it. It is kept beside the page, not in the panel: a person who
/// looked at another chat and came back is not shown the same App again.
const shownThrough = new Map<string, number>();

export function ChatApps({
  chat,
  agent,
  name,
  brought,
  shown,
  onShow,
  onHide,
  platform = "web",
  displayMode = "inline",
}: {
  chat: string;
  agent: string;
  name: string;
  /// The App a tool of the agent brought while the page was open.
  brought: Brought | undefined;
  shown: boolean;
  onShow: () => void;
  onHide: () => void;
  /// Where this page is, as the host tells an App: the Workbench is `web`,
  /// the page inside a messenger `mobile`. Only Apps that declared they
  /// work here open here.
  platform?: Platform;
  /// How an App is drawn here: beside the chat, or filling the place.
  displayMode?: "inline" | "fullscreen";
}) {
  const container = useRef<HTMLDivElement>(null);
  const mounted = useRef<{ app: MountedApp; opened: OpenedApp; connection: string } | null>(null);
  const [attachments, setAttachments] = useState<AppAttachment[] | null>(null);
  const [status, setStatus] = useState("");
  const [open, setOpen] = useState<OpenedApp | null>(null);
  const [form, setForm] = useState<{ server: string; tool: AppTool } | null>(null);

  const close = async () => {
    const closing = mounted.current;
    mounted.current = null;
    setOpen(null);
    // What was said about an App is said about that App only.
    setStatus("");
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
    const app = await window.SwemAppsBridge.mount({ container: container.current, opened, relay, observation, onStatus: setStatus, platform, displayMode });
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
    const key = `${chat} ${agent}`;
    if (!brought || brought.place <= (shownThrough.get(key) ?? 0)) return;
    shownThrough.set(key, brought.place);
    void (async () => {
      try {
        const connection = await sessionOf(chat, agent, false);
        if (!connection) return;
        const observed = await fetchJson<Observed | null>(
          `/api/connections/${part(connection)}/apps/observations/next?after=${brought.call - 1}&wait_ms=2000`,
        );
        // The session counts its calls from one: a call of an earlier
        // session is not this one.
        if (!observed || observed.observation.cursor !== brought.call) return;
        // An App that did not declare it works here is not opened here.
        if (!fits(observed.opened, platform)) {
          setStatus(`${named(observed.opened)} works on the Workbench, not here.`);
          return;
        }
        onShow();
        await mount(connection, observed.opened, observed.observation);
      } catch (error) {
        setStatus((error as Error).message);
      }
    })();
    // `mount` and `onShow` are this component's own and change with every draw.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [brought?.place, chat, agent]);

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

  const apps = (attachments ?? []).flatMap((attachment) =>
    attachment.apps.map((app) => ({ server: attachment.server_name, app, forms: forAPerson(attachment.tools, app.uri), here: fits(app, platform) })),
  );
  const elsewhere = apps.filter((one) => !one.here);
  const formsOf = (server: string, forms: AppTool[]) =>
    forms.map((tool) => (
      <button
        type="button"
        className="k-btn k-quiet"
        key={`${server} ${tool.name}`}
        title={tool.description ?? tool.name}
        onClick={() => {
          setStatus("");
          setForm({ server, tool });
        }}
      >
        {toolNamed(tool)}…
      </button>
    ));
  const openForms = open ? (apps.find((one) => one.server === open.server_name && one.app.uri === open.uri)?.forms ?? []) : [];
  return (
    <aside className={`w-side w-right w-apps${open ? " w-wide" : ""}`} aria-label={`Apps of ${name}`} hidden={!shown}>
      <div className="k-spread">
        <span className="k-eyebrow">Apps of {name}</span>
        <button type="button" className="k-btn k-quiet" aria-label="Hide the Apps" onClick={onHide}>
          <Cross size={14} />
        </button>
      </div>
      {open ? (
        <>
          <div className="k-spread">
            <span className="k-name w-one-line">{named(open)}</span>
            <button type="button" className="k-btn" onClick={() => void close()}>
              Close it
            </button>
          </div>
          {openForms.length > 0 ? <div className="k-inline w-tight w-wrap">{formsOf(open.server_name, openForms)}</div> : null}
        </>
      ) : (
        <div className="k-rail-group">
          {apps
            .filter((one) => one.here)
            .map(({ server, app, forms }) => (
              <div className="k-stack w-close" key={`${server} ${app.uri}`} data-app={app.uri}>
                <button type="button" className="k-rail-item" title={app.uri} onClick={() => void openApp(server, app.uri)}>
                  <Tiles />
                  <span className="k-two">
                    <span className="k-name">{named(app)}</span>
                    <span className="k-caption">{server}</span>
                  </span>
                </button>
                {forms.length > 0 ? <div className="k-inline w-tight w-wrap">{formsOf(server, forms)}</div> : null}
              </div>
            ))}
          {elsewhere.length > 0 ? (
            <div className="k-stack w-close" data-apps-elsewhere="">
              <span className="k-eyebrow">On the Workbench</span>
              {elsewhere.map(({ server, app }) => (
                <span className="k-caption" key={`${server} ${app.uri}`} data-app={app.uri}>
                  {named(app)} · {server}
                </span>
              ))}
            </div>
          ) : null}
          {attachments !== null && apps.length === 0 ? <span className="k-caption">The servers it attaches bring no Apps.</span> : null}
        </div>
      )}
      {form ? (
        <AppForm
          key={`${form.server} ${form.tool.name}`}
          connection={() => sessionOf(chat, agent, true)}
          server={form.server}
          tool={form.tool}
          onDone={(said) => {
            setForm(null);
            setStatus(said);
          }}
        />
      ) : null}
      {status ? <span className="k-caption">{status}</span> : null}
      <div className="w-app" ref={container} />
    </aside>
  );
}
