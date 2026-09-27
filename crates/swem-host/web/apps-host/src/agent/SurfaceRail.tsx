// The right rail: the Apps a session's MCP servers declare, and the one
// that is open. The bridge bundle mounts into a container React never
// re-renders, so the sandbox frame keeps its identity across state changes.

import { useEffect, useRef } from "react";

import { loadBridge, type MountedApp } from "./bridge.ts";
import type { OpenedApp } from "./session.ts";
import { sessionStore, useSession } from "./store.ts";

export function SurfaceRail() {
  const state = useSession();
  const container = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const mount = async (opened: OpenedApp, relay: (message: unknown) => Promise<unknown>, observation: unknown): Promise<MountedApp | null> => {
      if (!(await loadBridge()) || !window.SwemAppsBridge || !container.current) {
        sessionStore.setAppsStatus("Surfaces are off in this build; the same actions stay available as tools");
        return null;
      }
      return window.SwemAppsBridge.mount({
        container: container.current,
        opened,
        relay,
        observation,
        onStatus: (text) => sessionStore.setAppsStatus(text),
      });
    };
    sessionStore.mountObserved = mount;
    return () => {
      sessionStore.mountObserved = null;
    };
  }, []);
  /// What a surface is called when its server gives it no title: the last
  /// segment of its uri, as words.
  const surfaceName = (uri: string) => (uri.split("/").pop() ?? uri).replace(/[-_]/g, " ");
  const openApp = (serverName: string, uri: string) =>
    void sessionStore
      .openApp(serverName, uri, (opened, relay, observation) => sessionStore.mountObserved?.(opened, relay, observation) ?? Promise.resolve(null))
      .catch((error: Error) => sessionStore.setAppsStatus(`app open failed: ${error.message}`));
  const connected = state.connectionId !== null && state.routeId !== null;
  const context = state.context;
  return (
    <aside className="context" aria-label="Interactive surfaces">
      {/* What the open session is given with its next turns, as the host
          holds it. An App says it - what a person is looking at in a
          server's space - and here it is seen and let go of. */}
      <h2>What the agent is given</h2>
      <div id="session-context" className="k-caption" data-server-name={context?.server_name ?? ""} data-blocks={context?.content.length ?? 0}>
        {!connected ? (
          <span className="k-muted">no session open</span>
        ) : context ? (
          <>
            <strong>{context.server_name}</strong>
            <ul className="context-blocks">
              {context.content.map((block, index) => (
                <li key={index} className="context-block" data-kind={block.type}>
                  {block.type === "text" ? block.text.slice(0, 160) : block.name}
                </li>
              ))}
            </ul>
          </>
        ) : (
          <span className="k-muted">nothing yet - a server's space can say what you are looking at there</span>
        )}
      </div>
      {connected && context ? (
        <button id="session-context-clear" className="k-pick" onClick={() => void sessionStore.clearContext()}>
          Let go of it
        </button>
      ) : null}
      {state.contextError ? <div id="session-context-error" className="bad">{state.contextError}</div> : null}
      <h2>Surfaces</h2>
      <div id="apps-status">{state.appsStatus}</div>
      <div id="apps-list">
        {state.apps.flatMap((attachment) =>
          attachment.apps.map((app) => (
            <div className="app-row k-card" data-server={attachment.server_name} data-uri={app.uri} key={`${attachment.server_name} ${app.uri}`}>
              {/* The surface by what it is; the tools it carries behind a fold,
                  by name, for whoever wants them. The uri and scope stay as
                  data on the row. */}
              <strong title={app.uri}>{(app as { title?: string; name?: string }).title ?? (app as { name?: string }).name ?? surfaceName(app.uri)}</strong>{" "}
              <span className="k-muted" title={attachment.connection_scope}>{attachment.server_name}</span>{" "}
              <button className="app-open" onClick={() => openApp(attachment.server_name, app.uri)}>
                Open
              </button>
              <details className="app-tools">
                <summary className="k-muted">{attachment.tools.length} tools</summary>
                <ul className="app-tool-list">
                  {attachment.tools.map((tool) => (
                    <li key={tool.name} title={tool.description ?? tool.name}>
                      <code>{tool.name}</code> <span className="k-muted">{tool.visibility.join(", ")}</span>
                    </li>
                  ))}
                </ul>
              </details>
              {attachment.tools
                .filter((tool) => tool.resource_uri === app.uri && tool.visibility.includes("app") && !tool.visibility.includes("model"))
                .map((tool) => (
                  <button
                    className="structured-open"
                    title={tool.description ?? tool.name}
                    key={tool.name}
                    onClick={() => sessionStore.openStructuredInteraction(attachment.server_name, tool)}
                  >
                    {tool.title ?? "Structured fallback"}
                  </button>
                ))}
            </div>
          )),
        )}
      </div>
      <button id="app-close" disabled={!state.appOpen} onClick={() => void sessionStore.closeApp()}>
        Close app
      </button>
      <div id="app-container" ref={container} />
    </aside>
  );
}
