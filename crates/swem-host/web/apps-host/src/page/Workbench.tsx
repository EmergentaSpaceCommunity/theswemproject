// The Workbench: a rail, and whatever the person went to.

import { useEffect, useState, type ReactNode } from "react";
import { useStore } from "zustand";

import { sessionStore } from "../agent/store.ts";
import { Guard } from "../Guard.tsx";
import { ServerApp } from "../spaces/ServerApp.tsx";
import { StoreSpace } from "../store/StoreSpace.tsx";
import { AgentView, GroupView } from "./AgentView.tsx";
import { NewAgent } from "./NewAgent.tsx";
import { NewChat } from "./NewChat.tsx";
import { go, usePlace } from "./place.ts";
import { Providers } from "./Providers.tsx";
import { Rail, useSpaces } from "./Rail.tsx";
import { follow, world } from "./world.ts";

function Home() {
  const participants = useStore(world, (state) => state.participants);
  const ready = useStore(world, (state) => state.ready);
  const first = Object.values(participants)
    .filter((one) => one.kind === "agent" && !one.retired)
    .sort((left, right) => left.name.localeCompare(right.name))[0];
  useEffect(() => {
    if (!ready) return;
    // The product opens on somebody to talk to; with nobody yet, on making one.
    if (first) go({ at: "agent", agent: first.participant_id, tab: "chat" });
    else go({ at: "new-agent" });
  }, [ready, first]);
  return <main className="w-main" />;
}

export function Workbench() {
  const place = usePlace();
  const spaces = useSpaces();
  const ready = useStore(world, (state) => state.ready);
  const lost = useStore(world, (state) => state.lost);
  const participants = useStore(world, (state) => state.participants);
  const chats = useStore(world, (state) => state.chats);
  const [newChat, setNewChat] = useState(false);
  // The spaces a person has been to. An App that was opened stays open
  // while they are elsewhere: what they chose in it, what it plays and
  // what it said they are looking at are there when they come back.
  const [been, setBeen] = useState<string[]>([]);
  const at = place.at === "app" ? place.server : "";
  useEffect(() => {
    if (at) setBeen((known) => (known.includes(at) ? known : [...known, at]));
  }, [at]);
  useEffect(() => {
    follow();
    void sessionStore.loadProfiles();
    void sessionStore.loadEnvironments();
  }, []);
  let main: ReactNode = <Home />;
  if (ready) {
    if (place.at === "agent") {
      const agent = participants[place.agent];
      main = agent ? <AgentView agent={agent} tab={place.tab} chat={place.chat} key={agent.participant_id} /> : <Home />;
    } else if (place.at === "chat") {
      const chat = chats[place.chat];
      main = chat ? <GroupView chat={chat} key={chat.chat_id} /> : <Home />;
    } else if (place.at === "new-agent") {
      main = <NewAgent />;
    } else if (place.at === "providers") {
      main = <Providers tab={place.tab} />;
    } else if (place.at === "store") {
      main = (
        <main className="w-main">
          <Guard what="The Store">
            <StoreSpace hidden={false} />
          </Guard>
        </main>
      );
    } else if (place.at === "app") {
      // Drawn below, among the spaces that stay open.
      main = spaces.some((one) => one.server === place.server) ? null : <main className="w-main" />;
    }
  }
  return (
    <div className="w-shell">
      <Rail spaces={spaces} onNewChat={() => setNewChat(true)} />
      <div className="w-stage">
        {lost ? <div className="k-notice k-warning w-lost">The Workbench is not answering. What is here is what was last heard; it comes back by itself.</div> : null}
        {main}
        {spaces
          .filter((space) => space.server === at || been.includes(space.server))
          .map((space) => (
            <Guard what={`The space of ${space.name}`} key={space.server}>
              <main className="server-space w-main" data-server={space.server} hidden={space.server !== at}>
                <ServerApp server={space.server} uri={space.uri} />
              </main>
            </Guard>
          ))}
      </div>
      <NewChat open={newChat} onClose={() => setNewChat(false)} />
    </div>
  );
}
