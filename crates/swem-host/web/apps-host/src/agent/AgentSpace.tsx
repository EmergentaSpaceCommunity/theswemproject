// The Agent space: rail, conversation, surfaces. Every element a person
// touches here is a component over `SessionStore`; nothing binds by id.
//
// The main area has three panels, because configuring an agent, signing it in
// and talking to it are three different things a person does with it, and all
// three belong to the agent rather than to a project.

import { useEffect, useState } from "react";

import { AgentEnvironment } from "./AgentEnvironment.tsx";
import { AgentFiles } from "./AgentFiles.tsx";
import { Composer } from "./Composer.tsx";
import { ConversationHeader } from "./ConversationHeader.tsx";
import { ConversationView } from "./ConversationView.tsx";
import { DevPanel } from "./DevPanel.tsx";
import { Onboarding } from "./Onboarding.tsx";
import { SessionRail } from "./SessionRail.tsx";
import { sessionStore, useSession } from "./store.ts";
import { SurfaceRail } from "./SurfaceRail.tsx";
import { TerminalPanel } from "./TerminalPanel.tsx";

type Panel = "conversation" | "environment" | "files" | "terminal";

// The panel names are the data attributes the walks know; the captions are
// the person's words. "Setup" is where an agent is given its model, its role,
// its tools and its place to run - "Environment" named one of those.
const PANELS: { panel: Panel; label: string }[] = [
  { panel: "conversation", label: "Conversation" },
  { panel: "environment", label: "Setup" },
  { panel: "files", label: "Files" },
  { panel: "terminal", label: "Terminal" },
];

export function AgentSpace({ hidden }: { hidden: boolean }) {
  const state = useSession();
  const [panel, setPanel] = useState<Panel>("conversation");
  useEffect(() => {
    void sessionStore.loadProfiles();
  }, []);
  const configured = state.profiles.length > 0;
  // The header names whose agent this is, not the space: the space is
  // already named on the bar above.
  const current = state.profiles.find((profile) => profile.profile_id === state.profileId) ?? null;
  const agentName = state.onboarding?.agents.find((agent) => agent.agent_id === current?.agent_id)?.name;
  return (
    <div className="space agent-space" data-workbench-space="agent" hidden={hidden}>
      <SessionRail />
      <main className="main">
        <header className="main-header">
          <strong id="agent-title">{current ? `${current.profile_id}${agentName ? ` · ${agentName}` : ""}` : "Your agent"}</strong>
          <nav className="agent-panels" aria-label="Agent panels">
            {PANELS.map((entry) => (
              <button
                className={`k-pick${panel === entry.panel ? " k-active active" : ""}`}
                data-agent-tab={entry.panel}
                disabled={!configured}
                key={entry.panel}
                onClick={() => setPanel(entry.panel)}
              >
                {entry.label}
              </button>
            ))}
          </nav>
          <span id="turn-outcome">{state.conversation.turnOutcome}</span>
        </header>
        {configured ? null : <Onboarding />}
        <ConversationHeader hidden={!configured || panel !== "conversation"} />
        <ConversationView hidden={!configured || panel !== "conversation"} />
        <Composer hidden={!configured || panel !== "conversation"} />
        <AgentEnvironment hidden={!configured || panel !== "environment"} />
        <AgentFiles hidden={!configured || panel !== "files"} />
        <TerminalPanel hidden={!configured || panel !== "terminal"} />
      </main>
      <SurfaceRail />
      <DevPanel />
    </div>
  );
}
