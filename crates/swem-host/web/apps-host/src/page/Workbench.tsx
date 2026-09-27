// The Workbench: a rail, and whatever the person went to.

import { useEffect, useState } from "react";
import { useStore } from "zustand";

import { sessionStore } from "../agent/store.ts";
import { Guard } from "../Guard.tsx";
import { ServerApp } from "../spaces/ServerApp.tsx";
import { StoreSpace } from "../store/StoreSpace.tsx";
import { AgentView, GroupView } from "./AgentView.tsx";
import { NewAgent } from "./NewAgent.tsx";
import { NewChat } from "./NewChat.tsx";
import { go, usePlace } from "./place.ts";
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
  useEffect(() => {
    follow();
    void sessionStore.loadProfiles();
    void sessionStore.loadEnvironments();
  }, []);
  let main = <Home />;
  if (ready) {
    if (place.at === "agent") {
      const agent = participants[place.agent];
      main = agent ? <AgentView agent={agent} tab={place.tab} chat={place.chat} key={agent.participant_id} /> : <Home />;
    } else if (place.at === "chat") {
      const chat = chats[place.chat];
      main = chat ? <GroupView chat={chat} key={chat.chat_id} /> : <Home />;
    } else if (place.at === "new-agent") {
      main = <NewAgent />;
    } else if (place.at === "store") {
      main = (
        <main className="w-main">
          <Guard what="The Store">
            <StoreSpace hidden={false} />
          </Guard>
        </main>
      );
    } else if (place.at === "app") {
      const space = spaces.find((one) => one.server === place.server);
      main = space ? (
        <Guard what={`The space of ${space.name}`}>
          <main className="server-space w-main" data-server={space.server}>
            <ServerApp server={space.server} uri={space.uri} key={space.server} />
          </main>
        </Guard>
      ) : (
        <main className="w-main" />
      );
    }
  }
  return (
    <div className="w-shell">
      <Rail spaces={spaces} onNewChat={() => setNewChat(true)} />
      <div className="w-stage">
        {lost ? <div className="k-notice k-warning w-lost">The Workbench is not answering. What is here is what was last heard; it comes back by itself.</div> : null}
        {main}
      </div>
      <NewChat open={newChat} onClose={() => setNewChat(false)} />
    </div>
  );
}
