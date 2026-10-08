// The rail: who there is, what there is, where to go.

import { useEffect, useState } from "react";
import { useStore } from "zustand";

import { fetchJson } from "../http.ts";
import { HOSTS_CHANGED, named, type HostShown, type HostsShown } from "./door.ts";
import { ChatSign, Gear, Moon, People, Plug, Plus, Server, Shop, Sun, Tiles } from "./icons.tsx";
import { go, usePlace, type Place } from "./place.ts";
import type { Now, Participant } from "./types.ts";
import { Avatar, names, StateDot, useDoing, useStanding } from "./who.tsx";
import { chatsOf, world } from "./world.ts";

export interface SpaceView {
  server: string;
  uri: string;
  name: string;
  description?: string;
}

/// The spaces declared servers offer. A server installed from the Store
/// arrives while the page is open, so the list is read again now and then
/// and whenever the page comes back into view.
export function useSpaces(): SpaceView[] {
  const [spaces, setSpaces] = useState<SpaceView[]>([]);
  useEffect(() => {
    let left = false;
    const load = () => {
      fetchJson<SpaceView[]>("/api/spaces")
        .then((rows) => {
          if (!left) setSpaces(rows);
        })
        .catch(() => {});
    };
    load();
    const every = window.setInterval(load, 10000);
    const seen = () => {
      if (document.visibilityState === "visible") load();
    };
    document.addEventListener("visibilitychange", seen);
    return () => {
      left = true;
      window.clearInterval(every);
      document.removeEventListener("visibilitychange", seen);
    };
  }, []);
  return spaces;
}

/// Another host of the person's, with what is on it: read through this
/// host over the link between them, now and then, as the spaces are.
export interface OnAHost {
  host: HostShown;
  agents: Participant[];
  chats: Now["chats"];
  reached: boolean;
}

export function useOtherHosts(): OnAHost[] {
  const [hosts, setHosts] = useState<OnAHost[]>([]);
  useEffect(() => {
    let left = false;
    const load = async () => {
      let shown: HostsShown;
      try {
        shown = await fetchJson<HostsShown>("/api/hosts");
      } catch {
        // Not the owner, or a Workbench without hosts: nothing to show.
        return;
      }
      const on = await Promise.all(
        shown.hosts.map(async (host): Promise<OnAHost> => {
          try {
            const now = await fetchJson<Now>(`/api/hosts/${encodeURIComponent(host.host_id)}/api/now`);
            const agents = now.participants.filter((one) => one.kind === "agent" && !one.retired).sort((left, right) => left.name.localeCompare(right.name));
            const chats = now.chats.filter((chat) => chat.members.filter((member) => member.kind === "agent").length > 1 || chat.members.some((member) => member.kind === "guest" && !member.retired));
            return { host, agents, chats, reached: true };
          } catch {
            return { host, agents: [], chats: [], reached: false };
          }
        }),
      );
      if (!left) setHosts(on);
    };
    void load();
    const every = window.setInterval(() => void load(), 10000);
    const seen = () => {
      if (document.visibilityState === "visible") void load();
    };
    const changed = () => void load();
    document.addEventListener("visibilitychange", seen);
    window.addEventListener(HOSTS_CHANGED, changed);
    return () => {
      left = true;
      window.clearInterval(every);
      document.removeEventListener("visibilitychange", seen);
      window.removeEventListener(HOSTS_CHANGED, changed);
    };
  }, []);
  return hosts;
}

type Theme = "dark" | "light";
const THEME_KEY = "swem.workbench.theme";

function kept(): Theme {
  try {
    return localStorage.getItem(THEME_KEY) === "light" ? "light" : "dark";
  } catch {
    return "dark";
  }
}

export function ThemeSwitch() {
  const [theme, setTheme] = useState<Theme>(kept);
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem(THEME_KEY, theme);
    } catch {
      // Storage is a convenience; the theme still changes.
    }
  }, [theme]);
  return (
    <div className="k-segmented" role="group" aria-label="Theme">
      <button type="button" aria-label="Light theme" aria-pressed={theme === "light"} onClick={() => setTheme("light")}>
        <Sun size={14} />
      </button>
      <button type="button" aria-label="Dark theme" aria-pressed={theme === "dark"} onClick={() => setTheme("dark")}>
        <Moon size={14} />
      </button>
    </div>
  );
}

function AgentRow({ agent, active }: { agent: Participant; active: boolean }) {
  const what = useDoing(agent.participant_id);
  const { engine, host } = useStanding(agent);
  return (
    <button
      type="button"
      className={`k-rail-item${active ? " k-active" : ""}`}
      aria-current={active ? "page" : undefined}
      onClick={() => go({ at: "agent", agent: agent.participant_id, tab: "chat" })}
    >
      <Avatar who={agent} />
      <span className="k-two">
        <span className="k-name">{agent.name}</span>
        <span className="k-caption">{[engine, host].filter(Boolean).join(" · ")}</span>
      </span>
      <StateDot what={what} />
    </button>
  );
}

const here = (place: Place, agent: string): boolean => place.at === "agent" && place.agent === agent;

/// What is on another host of the person's, under its name. An agent or a
/// chat there opens through this host.
function OnHost({ on, place }: { on: OnAHost; place: Place }) {
  const { host, agents, chats, reached } = on;
  const there = (inside: string): boolean => place.at === "host" && place.host === host.host_id && place.inside === inside;
  const open = (inside: string) => go({ at: "host", host: host.host_id, inside });
  return (
    <div className="k-rail-group" data-on-host={host.name}>
      <span className="k-eyebrow k-inline w-tight">
        <Server size={12} />
        <span>On {host.name}</span>
      </span>
      {!reached ? <span className="k-caption w-rail-note">Out of reach</span> : null}
      {agents.map((agent) => (
        <button
          type="button"
          className={`k-rail-item${there(`agents/${encodeURIComponent(agent.participant_id)}`) ? " k-active" : ""}`}
          aria-current={there(`agents/${encodeURIComponent(agent.participant_id)}`) ? "page" : undefined}
          key={agent.participant_id}
          onClick={() => open(`agents/${encodeURIComponent(agent.participant_id)}`)}
        >
          <Avatar who={agent} />
          <span className="k-two">
            <span className="k-name">{agent.name}</span>
            <span className="k-caption">On {host.name}</span>
          </span>
        </button>
      ))}
      {chats.map((chat) => (
        <button type="button" className={`k-rail-item${there(`chats/${encodeURIComponent(chat.chat_id)}`) ? " k-active" : ""}`} key={chat.chat_id} onClick={() => open(`chats/${encodeURIComponent(chat.chat_id)}`)}>
          <span className="k-avatar">
            <People />
          </span>
          <span className="k-two">
            <span className="k-name">{chat.title || "New chat"}</span>
            <span className="k-caption">On {host.name}</span>
          </span>
        </button>
      ))}
      {reached ? (
        <>
          <button type="button" className={`k-rail-item k-new${there("agents/new") ? " k-active" : ""}`} onClick={() => open("agents/new")}>
            <Plus />
            <span>New agent</span>
          </button>
          <button type="button" className={`k-rail-item k-quiet${there("store") ? " k-active" : ""}`} onClick={() => open("store")}>
            <Shop size={17} />
            <span>Store on {host.name}</span>
          </button>
        </>
      ) : null}
    </div>
  );
}

export function Rail({ spaces, hosts, onNewChat }: { spaces: SpaceView[]; hosts: OnAHost[]; onNewChat: () => void }) {
  const place = usePlace();
  const called = useStore(named, (state) => state.called);
  const owner = useStore(world, (state) => state.owner);
  const you = useStore(world, (state) => state.you);
  const participants = useStore(world, (state) => state.participants);
  const several = useStore(world, (state) => chatsOf(state, null).map((chat) => chat.chat_id).join(" "));
  const chats = useStore(world, (state) => state.chats);
  const agents = Object.values(participants)
    .filter((one) => one.kind === "agent" && !one.retired)
    .sort((left, right) => left.name.localeCompare(right.name));
  return (
    <nav className="k-rail w-rail" aria-label="Workbench">
      <div className="w-brand">
        <span className="w-brand-mark" />
        <span>{called}</span>
      </div>
      <div className="k-rail-group">
        <span className="k-eyebrow">Agents</span>
        {agents.map((agent) => (
          <AgentRow agent={agent} active={here(place, agent.participant_id)} key={agent.participant_id} />
        ))}
        <button type="button" className={`k-rail-item k-new${place.at === "new-agent" ? " k-active" : ""}`} onClick={() => go({ at: "new-agent" })}>
          <Plus />
          <span>New agent</span>
        </button>
      </div>
      <div className="k-rail-group">
        <span className="k-eyebrow">Chats</span>
        {several
          .split(" ")
          .filter(Boolean)
          .map((chatId) => {
            const chat = chats[chatId];
            if (!chat) return null;
            const active = place.at === "chat" && place.chat === chatId;
            return (
              <button type="button" className={`k-rail-item${active ? " k-active" : ""}`} key={chatId} onClick={() => go({ at: "chat", chat: chatId })}>
                <span className="k-avatar">
                  <People />
                </span>
                <span className="k-two">
                  <span className="k-name">{chat.title || "New chat"}</span>
                  <span className="k-caption">{names(chat.members, you)}</span>
                </span>
              </button>
            );
          })}
        <button type="button" className="k-rail-item k-new" onClick={onNewChat} disabled={agents.length < 2} title={agents.length < 2 ? "A chat of several needs two agents" : undefined}>
          <Plus />
          <span>New chat</span>
        </button>
      </div>
      {hosts.map((on) => (
        <OnHost on={on} place={place} key={on.host.host_id} />
      ))}
      {spaces.length > 0 ? (
        <div className="k-rail-group">
          <span className="k-eyebrow">Apps</span>
          {spaces.map((space) => {
            const active = place.at === "app" && place.server === space.server;
            return (
              <button
                type="button"
                className={`k-rail-item k-quiet${active ? " k-active" : ""}`}
                title={space.description ?? undefined}
                key={space.server}
                onClick={() => go({ at: "app", server: space.server })}
              >
                <Tiles size={17} />
                <span>{space.name}</span>
              </button>
            );
          })}
        </div>
      ) : null}
      <div className="w-grow" />
      <div className="k-rail-group">
        <button
          type="button"
          className={`k-rail-item k-quiet${place.at === "providers" ? " k-active" : ""}`}
          onClick={() => go({ at: "providers", tab: "models" })}
        >
          <Plug size={17} />
          <span>Providers</span>
        </button>
        <button type="button" className={`k-rail-item k-quiet${place.at === "store" ? " k-active" : ""}`} onClick={() => go({ at: "store" })}>
          <Shop size={17} />
          <span>Store</span>
        </button>
        <button type="button" className={`k-rail-item k-quiet${place.at === "settings" ? " k-active" : ""}`} onClick={() => go({ at: "settings", tab: "access" })}>
          <Gear size={17} />
          <span>Settings</span>
        </button>
      </div>
      <div className="k-spread w-you">
        <span className="k-inline w-tight">
          <Avatar who={owner ?? undefined} size="small" />
          <span className="k-two">
            <span className="k-name">{owner?.name ?? ""}</span>
            <span className="k-mono k-muted">{owner ? `@${owner.handle}` : ""}</span>
          </span>
        </span>
        <ThemeSwitch />
      </div>
    </nav>
  );
}

export { ChatSign };
