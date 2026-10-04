// The Workbench: a rail, and whatever the person went to.

import { useEffect, useState, type ReactNode } from "react";
import { useStore } from "zustand";

import { sessionStore } from "../agent/store.ts";
import { Guard } from "../Guard.tsx";
import { ServerApp } from "../spaces/ServerApp.tsx";
import { StoreSpace } from "../store/StoreSpace.tsx";
import { AgentView, GroupView } from "./AgentView.tsx";
import { inAMessenger } from "./door.ts";
import { pointedAt } from "./messenger.ts";
import { NewAgent } from "./NewAgent.tsx";
import { NewChat } from "./NewChat.tsx";
import { go, usePlace, type AgentTab } from "./place.ts";
import { Providers } from "./Providers.tsx";
import { Rail, useSpaces } from "./Rail.tsx";
import { Settings } from "./Settings.tsx";
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

/// The page for somebody who came through a messenger: no rail, the bot's
/// agent alone, with the tabs that are theirs - Chat, and for the owner
/// Files and Apps - opened on what the bot pointed at.
function InTheMessenger({ agent, owner }: { agent: string; owner: boolean }) {
  const place = usePlace();
  const ready = useStore(world, (state) => state.ready);
  const participants = useStore(world, (state) => state.participants);
  const questions = useStore(world, (state) => state.questions);
  const lost = useStore(world, (state) => state.lost);
  const [pointed] = useState(() => pointedAt());
  const tabs: AgentTab[] = owner ? ["chat", "files", "apps"] : ["chat"];
  // The stream and nothing else: the profiles and the machines are the
  // Workbench's own, not a messenger session's to ask for.
  useEffect(() => {
    follow();
  }, []);
  // Where the bot pointed, once: a tab, or the chat a question waits in.
  const [went, setWent] = useState(false);
  useEffect(() => {
    if (!ready || went) return;
    const open = pointed?.open ?? "chat";
    if (open === "question" && pointed?.question) {
      const question = questions[pointed.question];
      if (!question) return;
      setWent(true);
      go({ at: "agent", agent, tab: "chat", chat: question.chat_id });
      return;
    }
    setWent(true);
    const tab: AgentTab = (open === "files" || open === "apps") && owner ? open : "chat";
    go({ at: "agent", agent, tab, ...(pointed?.chat ? { chat: pointed.chat } : {}) });
  }, [ready, went, pointed, questions, agent, owner]);
  const who = participants[agent];
  const tab: AgentTab = place.at === "agent" && tabs.includes(place.tab) ? place.tab : "chat";
  const chat = place.at === "agent" ? place.chat : undefined;
  return (
    <div className="w-shell w-in-messenger" data-in-messenger={owner ? "owner" : "guest"}>
      <div className="w-stage">
        {lost ? <div className="k-notice k-warning w-lost">The Workbench is not answering. What is here is what was last heard; it comes back by itself.</div> : null}
        {ready && who ? <AgentView agent={who} tab={tab} chat={chat} tabs={tabs} compact key={who.participant_id} /> : <main className="w-main" />}
      </div>
    </div>
  );
}

export function Workbench() {
  const messenger = useStore(inAMessenger, (state) => state.messenger);
  if (messenger) return <InTheMessenger agent={messenger.agent_id} owner={messenger.owner} />;
  return <WorkbenchWhole />;
}

function WorkbenchWhole() {
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
    } else if (place.at === "settings") {
      main = <Settings tab={place.tab} />;
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
