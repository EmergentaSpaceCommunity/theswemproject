import { StrictMode, useEffect, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";

import { Guard } from "./Guard.tsx";
import { toggleDev, useDev } from "./dev.ts";
import { AgentSpace } from "./agent/AgentSpace.tsx";
import { PermissionDialog } from "./agent/PermissionDialog.tsx";
import { StructuredDialog } from "./agent/StructuredDialog.tsx";
import { ProjectSpace } from "./project/ProjectSpace";
import { StoreSpace } from "./store/StoreSpace.tsx";

declare global {
  interface Window {
    __SWEM_WORKBENCH_WEB__?: string;
  }
}

type Space = "agent" | "project" | "store";
const SPACE_KEY = "swem.workbench.space";
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

function SpaceSwitcher({ space, onSelect }: { space: Space; onSelect: (space: Space) => void }) {
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
  const choose = (next: Space) => setSpace(next);
  useEffect(() => {
    try {
      localStorage.setItem(SPACE_KEY, space);
    } catch {
      // Storage is a convenience; the space still switches.
    }
  }, [space]);
  return (
    <StrictMode>
      <div className="app-shell" data-active-space={space}>
        <SpaceSwitcher space={space} onSelect={choose} />
        <Guard what="The Agent space">
          <AgentSpace hidden={space !== "agent"} />
        </Guard>
        <Guard what="The Project space">
          <ProjectSpace hidden={space !== "project"} />
        </Guard>
        <Guard what="The Store">
          <StoreSpace hidden={space !== "store"} />
        </Guard>
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
