import { StrictMode, useEffect, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";

import { Guard } from "./Guard.tsx";
import { toggleDev, useDev } from "./dev.ts";
import { AgentSpace } from "./agent/AgentSpace.tsx";
import { PermissionDialog } from "./agent/PermissionDialog.tsx";
import { StructuredDialog } from "./agent/StructuredDialog.tsx";
import { ProjectSpace } from "./project/ProjectSpace";
import { DomainApp } from "./project/DomainApps";
import { fetchJson } from "./project/api";
import { StoreSpace } from "./store/StoreSpace.tsx";

declare global {
  interface Window {
    __SWEM_WORKBENCH_WEB__?: string;
  }
}

/// The three spaces the host draws itself, and one per server that declares
/// a home App: a surface the server ships and the host shows as a space of
/// its own (`/api/spaces`), opened without a tool call.
type Space = "agent" | "project" | "store" | { server: string };
const SPACE_KEY = "swem.workbench.space";
interface SpaceView { server: string; uri: string; name: string; description?: string }
const sameSpace = (left: Space, right: Space): boolean =>
  typeof left === "string" || typeof right === "string" ? left === right : left.server === right.server;
/// The spaces declared servers offer. A server installed from the Store
/// arrives while the page is open, so the list is read again now and then
/// and whenever the page comes back into view; the host dials each server
/// once and keeps it, so a re-read costs nothing.
function useSpaces(): SpaceView[] {
  const [spaces, setSpaces] = useState<SpaceView[]>([]);
  useEffect(() => {
    let disposed = false;
    const load = () => {
      fetchJson<SpaceView[]>("/api/spaces")
        .then((rows) => { if (!disposed) setSpaces(rows); })
        .catch(() => {});
    };
    load();
    const every = window.setInterval(load, 10000);
    const onVisible = () => { if (document.visibilityState === "visible") load(); };
    document.addEventListener("visibilitychange", onVisible);
    return () => { disposed = true; window.clearInterval(every); document.removeEventListener("visibilitychange", onVisible); };
  }, []);
  return spaces;
}
type Theme = "dark" | "light";
const THEME_KEY = "swem.workbench.theme";

/// The product lands on the Project space: the ladder is the door for a
/// person and an agent alike, and the agent is set up from the project.
/// A person who chose the Agent space keeps it.
function loadSpace(): Space {
  try {
    const kept = localStorage.getItem(SPACE_KEY);
    return kept === "agent" || kept === "store" ? kept : "project";
  } catch {
    return "project";
  }
}

/// The theme a person picked, kept in this browser exactly as the space is.
/// It sat on the same bar as the space switch and kept nothing: a person who
/// chose Light got Dark back on every reload, because nothing here read or
/// wrote it. Storage is a convenience, so a browser that refuses it still
/// gets a theme - the product's own.
function loadTheme(): Theme {
  try {
    return localStorage.getItem(THEME_KEY) === "light" ? "light" : "dark";
  } catch {
    return "dark";
  }
}

function SpaceSwitcher({ space, spaces, onSelect }: { space: Space; spaces: SpaceView[]; onSelect: (space: Space) => void }) {
  const [theme, setTheme] = useState<Theme>(loadTheme);
  const dev = useDev();
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem(THEME_KEY, theme);
    } catch {
      // Storage is a convenience; the theme still changes.
    }
  }, [theme]);
  return (
    <nav id="space-switcher" className="space-bar" aria-label="Workbench spaces">
      <button id="space-agent" aria-pressed={space === "agent"} onClick={() => onSelect("agent")}>
        Agent space
      </button>
      <button id="space-project" aria-pressed={space === "project"} onClick={() => onSelect("project")}>
        Project space
      </button>
      <button id="space-store" aria-pressed={space === "store"} onClick={() => onSelect("store")}>
        Store
      </button>
      {spaces.map((offered) => (
        <button
          key={offered.server}
          id={`space-server-${offered.server}`}
          className="space-server"
          data-server={offered.server}
          title={offered.description ?? undefined}
          aria-pressed={sameSpace(space, { server: offered.server })}
          onClick={() => onSelect({ server: offered.server })}
        >
          {offered.name}
        </button>
      ))}
      <label style={{ marginLeft: "auto" }}>
        Theme{" "}
        <select
          id="theme"
          aria-label="Theme"
          value={theme}
          onChange={(event) => setTheme(event.target.value === "light" ? "light" : "dark")}
        >
          <option value="dark">Dark</option>
          <option value="light">Light</option>
        </select>
      </label>
      {/* The dev drawer: ids, the event log, raw resource shapes. Off unless
          asked for; `?dev=1` in the address asks for it too. */}
      <button id="dev-toggle" className="dev-toggle" aria-pressed={dev} title="Show what the page carries for whoever builds it" onClick={toggleDev}>
        Dev
      </button>
    </nav>
  );
}

function Workbench() {
  const [space, setSpace] = useState<Space>(loadSpace);
  const spaces = useSpaces();
  const choose = (next: Space) => setSpace(next);
  useEffect(() => {
    if (typeof space !== "string") return;
    try {
      localStorage.setItem(SPACE_KEY, space);
    } catch {
      // Storage is a convenience; the space still switches.
    }
  }, [space]);
  const opened = typeof space === "string" ? null : spaces.find((offered) => offered.server === space.server) ?? null;
  return (
    <StrictMode>
      <div className="app-shell" data-active-space={typeof space === "string" ? space : `server:${space.server}`}>
        <SpaceSwitcher space={space} spaces={spaces} onSelect={choose} />
        <Guard what="The Agent space">
          <AgentSpace hidden={space !== "agent"} />
        </Guard>
        <Guard what="The Project space">
          <ProjectSpace hidden={space !== "project"} />
        </Guard>
        <Guard what="The Store">
          <StoreSpace hidden={space !== "store"} />
        </Guard>
        {/* A server's own space: its home App, filling the main area, opened
            through the host's door for spaces rather than through a project. */}
        {opened ? (
          <Guard what={`The space of ${opened.server}`}>
            <main id="server-space" className="server-space" data-server={opened.server}>
              <DomainApp server={opened.server} uri={opened.uri} changed={null} openPath={`/api/spaces/${encodeURIComponent(opened.server)}/open`} />
            </main>
          </Guard>
        ) : null}
        <PermissionDialog />
        <StructuredDialog />
      </div>
    </StrictMode>
  );
}

const container = document.getElementById("workbench-root");
if (!container) throw new Error("Workbench mount is missing");

flushSync(() => createRoot(container).render(<Workbench />));
container.dataset.renderer = "react";
window.__SWEM_WORKBENCH_WEB__ = "react-19";
