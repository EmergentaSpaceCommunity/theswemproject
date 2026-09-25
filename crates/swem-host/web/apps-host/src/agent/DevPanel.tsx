// What the page carries for whoever builds it: the route and connection ids,
// the event log, the raw resource shapes a stock client also knows, the
// open surface's own account of itself, and who a turn is written as.
//
// None of it is for a person using their agent, so it lives in one drawer
// under the dev switch. The drawer is in the page whether the switch is on or
// not - hidden, not absent - because the walks read these ids by text and
// value, and a hidden element still has both.

import { useEffect, useRef } from "react";

import { useDev } from "../dev.ts";
import { linksIn } from "./events.ts";
import { sessionStore, useSession } from "./store.ts";

function EventRow({
  sequence,
  kind,
  source,
  payload,
  fromHistory,
}: {
  sequence: number;
  kind: string;
  source: string;
  payload: unknown;
  fromHistory: boolean;
}) {
  const links = linksIn(payload).map((link) => {
    let parsed: URL | null = null;
    try {
      parsed = new URL(link.href, location.href);
    } catch {
      parsed = null;
    }
    return { ...link, external: parsed !== null && ["http:", "https:"].includes(parsed.protocol), href: parsed?.href ?? link.href };
  });
  return (
    <li data-kind={kind} data-sequence={sequence} data-source={source} data-history={fromHistory ? "true" : undefined}>
      [{sequence}] {kind}
      {links.map((link, index) =>
        link.external ? (
          <a className="resource-link" href={link.href} rel="noopener noreferrer" key={index}>
            {" "}
            {link.label}
          </a>
        ) : (
          <span className="resource-chip k-chip k-roomy" key={index}>
            {" "}
            {link.label}
          </span>
        ),
      )}
    </li>
  );
}

export function DevPanel() {
  const dev = useDev();
  const state = useSession();
  // The route field is the person's own: a value put there by hand (or by a
  // driver) is read when a session is opened, and the store's choice is
  // written back into it when it changes.
  const routeField = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (routeField.current && routeField.current.value !== state.routeInput) routeField.current.value = state.routeInput;
  }, [state.routeInput]);
  const connected = state.connectionId !== null && state.routeId !== null;
  const terminal = state.conversation.terminal;
  const opened = state.opened;
  return (
    <aside id="dev-panel" className="dev-panel" aria-label="Developer diagnostics" hidden={!dev}>
      <div className="dev-panel-columns">
        <section>
          <div className="k-eyebrow">Session</div>
          <div className="legacy-inputs">
            <input id="route-id" placeholder="route id (load/resume)" ref={routeField} defaultValue={state.routeInput} />
            {/* Who is writing. The agent is told in the turn itself, because
                the protocol carries no author; leaving this empty is honest -
                the record then says the Workbench and not a person. */}
            <input
              id="writing-as"
              className="writing-as"
              placeholder="Writing as…"
              aria-label="Who is writing"
              defaultValue={state.writingAs}
              key={state.writingAs}
              onBlur={(event) => sessionStore.setWritingAs(event.target.value.trim())}
            />
          </div>
          <pre>
            connection <span id="connection-id">{connected ? state.connectionId : "-"}</span>
            {"\n"}route <span id="route">{state.routeId ?? "-"}</span>
            {terminal?.detail ? `\nterminal ${terminal.detail}` : ""}
          </pre>
        </section>
        <section>
          <div className="k-eyebrow">Resource shapes</div>
          <div className="legacy-inputs">
            <input id="link-uri" placeholder="resource link URI" />
            <input id="link-name" placeholder="resource name" />
            <input id="link-mime" placeholder="MIME type" />
            <input id="embed-uri" placeholder="embedded resource URI" />
            <input id="embed-text" placeholder="embedded text body" />
          </div>
        </section>
        <section>
          <div className="k-eyebrow">Open surface</div>
          <div className="k-caption">
            server: <span id="app-server">{opened?.server_name ?? "-"}</span> · resource: <span id="app-uri">{opened?.uri ?? "-"}</span> · permissions:{" "}
            <span id="app-permissions">{opened ? JSON.stringify(opened.permissions) : "-"}</span> · refusals:{" "}
            <span id="app-refusals">{state.conversation.refusals}</span>
          </div>
        </section>
      </div>
      <div className="k-eyebrow">Events</div>
      <ol id="events">
        {state.events.map((event) => (
          <EventRow
            key={event.sequence}
            sequence={event.sequence}
            kind={event.kind}
            source={event.source}
            payload={event.payload}
            fromHistory={state.historySequences.includes(event.sequence)}
          />
        ))}
      </ol>
    </aside>
  );
}
