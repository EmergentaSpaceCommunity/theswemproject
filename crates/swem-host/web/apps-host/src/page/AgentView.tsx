// An agent: who it is, what it stands on and where it lives, the chats it
// has, and its files, terminal and settings.

import { useEffect, useState } from "react";
import { useStore } from "zustand";

import { AgentSettings } from "./settings/AgentSettings.tsx";
import { sessionStore } from "../agent/store.ts";
import { Guard } from "../Guard.tsx";
import { ChatView, FirstWords } from "./ChatView.tsx";
import { ChatSign, Clock, Folder, Gear, Moon, People, Plug, Plus, Prompt } from "./icons.tsx";
import { AddSomeone, Members } from "./Members.tsx";
import { AgentChannels } from "./AgentChannels.tsx";
import { Files } from "./Files.tsx";
import { Terminals } from "./Terminals.tsx";
import { go, type AgentTab } from "./place.ts";
import { Schedules } from "./Schedules.tsx";
import type { Chat, Participant } from "./types.ts";
import { Avatar, names, StateWord, useDoing, useStanding, when } from "./who.tsx";
import { act, chatsOf, doing, world } from "./world.ts";

const TABS: { tab: AgentTab; label: string; sign: typeof ChatSign }[] = [
  { tab: "chat", label: "Chat", sign: ChatSign },
  { tab: "files", label: "Files", sign: Folder },
  { tab: "terminal", label: "Terminal", sign: Prompt },
  { tab: "schedules", label: "Schedules", sign: Clock },
  { tab: "channels", label: "Channels", sign: Plug },
  { tab: "settings", label: "Settings", sign: Gear },
];

function Header({ agent, tab }: { agent: Participant; tab: AgentTab }) {
  const what = useDoing(agent.participant_id);
  const { profile, engine, host } = useStanding(agent);
  const role = (profile?.role ?? "").split("\n").find((line) => line.trim()) ?? "";
  return (
    <header className="w-head">
      <div className="k-spread">
        <div className="k-inline w-nowrap">
          <Avatar who={agent} size="large" />
          <div className="w-col w-close">
            <div className="k-inline w-tight">
              <h1 className="w-h1">{agent.name}</h1>
              <span className="k-mono k-muted">@{agent.handle}</span>
              <StateWord what={what} />
            </div>
            {role ? <div className="k-caption w-one-line">{role}</div> : null}
          </div>
        </div>
        <div className="k-inline w-tight w-nowrap">
          {engine ? <span className="k-chip">{engine}</span> : null}
          {profile?.model ? <span className="k-chip">{profile.model}</span> : null}
          {host ? <span className="k-chip">{host}</span> : null}
          <button
            type="button"
            className="k-btn k-quiet"
            disabled={what !== "ready"}
            title="Its sessions are let go of; it wakes with the next thing said"
            onClick={() => void act.sleep(agent.participant_id)}
          >
            <Moon />
            <span>Put to sleep</span>
          </button>
        </div>
      </div>
      <nav className="k-tabs" aria-label={agent.name}>
        {TABS.map((one) => (
          <button
            type="button"
            className={`k-tab${tab === one.tab ? " k-active" : ""}`}
            aria-current={tab === one.tab ? "page" : undefined}
            key={one.tab}
            onClick={() => go({ at: "agent", agent: agent.participant_id, tab: one.tab })}
          >
            <one.sign size={15} />
            <span>{one.label}</span>
          </button>
        ))}
      </nav>
    </header>
  );
}

function ChatRow({ chat, agent, active }: { chat: Chat; agent: Participant; active: boolean }) {
  const owner = useStore(world, (state) => state.owner);
  const what = useStore(world, (state) => doing(state, agent.participant_id, chat.chat_id));
  const several = chat.members.filter((member) => member.kind === "agent").length > 1;
  const state = what === "working" ? "working now" : what === "asking" ? "waiting for you" : what === "waiting" ? "next in line" : when(chat.last_at_ms);
  return (
    <button
      type="button"
      className={`k-rail-item w-top${active ? " k-active" : ""}`}
      aria-current={active ? "page" : undefined}
      onClick={() => go({ at: "agent", agent: agent.participant_id, tab: "chat", chat: chat.chat_id })}
    >
      <span className="k-two">
        <span className={active ? "k-name" : undefined}>{chat.title || "New chat"}</span>
        <span className="k-inline k-caption w-tight w-nowrap">
          {several ? <People size={13} /> : <ChatSign size={13} />}
          <span className="w-one-line">{[names(chat.members, owner), state].filter(Boolean).join(" · ")}</span>
        </span>
      </span>
    </button>
  );
}

function Chats({ agent, chats, chosen }: { agent: Participant; chats: Chat[]; chosen: string | null }) {
  const alone = chats.filter((chat) => chat.members.filter((member) => member.kind === "agent").length === 1);
  const several = chats.filter((chat) => !alone.includes(chat));
  return (
    <aside className="w-side" aria-label={`Chats with ${agent.name}`}>
      <button type="button" className="k-btn w-wide" onClick={() => go({ at: "agent", agent: agent.participant_id, tab: "chat", chat: "new" })}>
        <Plus />
        <span>New chat with {agent.name}</span>
      </button>
      {alone.length > 0 ? (
        <div className="k-rail-group">
          <span className="k-eyebrow">Chats</span>
          {alone.map((chat) => <ChatRow chat={chat} agent={agent} active={chat.chat_id === chosen} key={chat.chat_id} />)}
        </div>
      ) : null}
      {several.length > 0 ? (
        <div className="k-rail-group">
          <span className="k-eyebrow">Group chats {agent.name} is in</span>
          {several.map((chat) => <ChatRow chat={chat} agent={agent} active={chat.chat_id === chosen} key={chat.chat_id} />)}
        </div>
      ) : null}
    </aside>
  );
}

export function AgentView({ agent, tab, chat }: { agent: Participant; tab: AgentTab; chat?: string }) {
  const order = useStore(world, (state) => chatsOf(state, agent.participant_id).map((one) => one.chat_id).join(" "));
  const known = useStore(world, (state) => state.chats);
  const chats = order.split(" ").filter(Boolean).flatMap((id) => (known[id] ? [known[id]] : []));
  // The files, the terminal and the settings are the agent's profile's.
  useEffect(() => {
    if (agent.profile_id) sessionStore.selectProfile(agent.profile_id);
  }, [agent.profile_id]);
  const alone = chats.find((one) => one.members.filter((member) => member.kind === "agent").length === 1);
  const chosen = chat === "new" ? null : ((chat && known[chat]) || alone || chats[0] || null);
  return (
    <main className="w-main">
      <Header agent={agent} tab={tab} />
      {tab === "chat" ? (
        <div className="w-body">
          <Chats agent={agent} chats={chats} chosen={chosen?.chat_id ?? null} />
          <Guard what="The chat">
            {chosen ? (
              <ChatView chat={chosen} key={chosen.chat_id} />
            ) : (
              <FirstWords agent={agent} key={agent.participant_id} onBegun={(begun) => go({ at: "agent", agent: agent.participant_id, tab: "chat", chat: begun.chat_id })} />
            )}
          </Guard>
        </div>
      ) : null}
      <div className="w-panel" hidden={tab === "chat"}>
        <Guard what="The agent's files">
          <Files agent={agent} hidden={tab !== "files"} key={agent.profile_id ?? agent.participant_id} />
        </Guard>
        <Guard what="The agent's terminal">
          <Terminals agent={agent} hidden={tab !== "terminal"} key={agent.profile_id ?? agent.participant_id} />
        </Guard>
        <Guard what="The agent's schedules">
          <Schedules agent={agent} hidden={tab !== "schedules"} />
        </Guard>
        <Guard what="The agent's channels">
          <AgentChannels agent={agent} hidden={tab !== "channels"} />
        </Guard>
        <Guard what="The agent's settings">
          <AgentSettings agent={agent} hidden={tab !== "settings"} />
        </Guard>
      </div>
    </main>
  );
}

/// A chat of several: what is said, and beside it who is in it and by
/// what rules.
export function GroupView({ chat }: { chat: Chat }) {
  const [adding, setAdding] = useState(false);
  const agents = chat.members.filter((member) => member.kind === "agent" && !member.retired).length;
  const people = chat.members.filter((member) => member.kind !== "schedule" && !member.retired).length;
  return (
    <main className="w-main">
      <header className="w-head w-head-plain">
        <div className="k-spread">
          <div className="k-inline w-nowrap">
            <span className="k-avatar k-large">
              <People size={20} />
            </span>
            <div className="w-col w-close">
              <h1 className="w-h1">{chat.title || "New chat"}</h1>
              <div className="k-caption">
                A chat of {people}. {agents > 1 ? (chat.answer_rule === "always" ? "Agents answer whatever is said." : "Agents answer when someone names them.") : "Its agent answers whatever is said."}
              </div>
            </div>
          </div>
          <button type="button" className="k-btn" onClick={() => setAdding(true)}>
            <Plus size={15} />
            <span>Add someone</span>
          </button>
        </div>
      </header>
      <div className="w-body">
        <Guard what="The chat">
          <ChatView chat={chat} key={chat.chat_id} />
        </Guard>
        <Members chat={chat} onAdd={() => setAdding(true)} />
      </div>
      <AddSomeone chat={chat} open={adding} onClose={() => setAdding(false)} />
    </main>
  );
}
