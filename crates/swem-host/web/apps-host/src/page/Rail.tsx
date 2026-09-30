// The rail: who there is, what there is, where to go.

import { useEffect, useState } from "react";
import { useStore } from "zustand";

import { fetchJson } from "../http.ts";
import { named } from "./door.ts";
import { ChatSign, Gear, Moon, People, Plug, Plus, Shop, Sun, Tiles } from "./icons.tsx";
import { go, usePlace, type Place } from "./place.ts";
import type { Participant } from "./types.ts";
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

export function Rail({ spaces, onNewChat }: { spaces: SpaceView[]; onNewChat: () => void }) {
  const place = usePlace();
  const called = useStore(named, (state) => state.called);
  const owner = useStore(world, (state) => state.owner);
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
                  <span className="k-caption">{names(chat.members, owner)}</span>
                </span>
              </button>
            );
          })}
        <button type="button" className="k-rail-item k-new" onClick={onNewChat} disabled={agents.length < 2} title={agents.length < 2 ? "A chat of several needs two agents" : undefined}>
          <Plus />
          <span>New chat</span>
        </button>
      </div>
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
