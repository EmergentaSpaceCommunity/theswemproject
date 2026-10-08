// Where in the product a person is. Kept in the address after the hash, so
// coming back, reloading and sending somebody the address all lead to the
// same place.

import { useSyncExternalStore } from "react";

export type AgentTab = "chat" | "files" | "terminal" | "schedules" | "channels" | "settings" | "apps";
export type ProvidersTab = "engines" | "models" | "environments" | "time" | "channels";
export type SettingsTab = "access" | "hosts";

export type Place =
  | { at: "home" }
  | { at: "agent"; agent: string; tab: AgentTab; chat?: string }
  | { at: "chat"; chat: string }
  | { at: "new-agent" }
  | { at: "providers"; tab: ProvidersTab }
  | { at: "store" }
  | { at: "settings"; tab: SettingsTab }
  | { at: "app"; server: string }
  /// Another host of the person's, drawn through this one, at a place of
  /// its own page: `agents/<id>`, `chats/<id>`, `store`, or its home.
  | { at: "host"; host: string; inside: string };

const TABS: AgentTab[] = ["chat", "files", "terminal", "schedules", "channels", "settings", "apps"];

export function read(hash: string): Place {
  const [first, second, third, fourth] = hash.replace(/^#\/?/, "").split("/").map(decodeURIComponent);
  if (first === "agents" && second === "new") return { at: "new-agent" };
  if (first === "agents" && second) {
    if (third === "chats" && fourth) return { at: "agent", agent: second, tab: "chat", chat: fourth };
    const tab = TABS.find((one) => one === third) ?? "chat";
    return { at: "agent", agent: second, tab };
  }
  if (first === "chats" && second) return { at: "chat", chat: second };
  if (first === "providers") {
    const tab = second === "engines" || second === "environments" || second === "time" || second === "channels" ? second : "models";
    return { at: "providers", tab };
  }
  if (first === "store") return { at: "store" };
  if (first === "settings") return { at: "settings", tab: second === "hosts" ? "hosts" : "access" };
  if (first === "hosts" && second) {
    const inside = hash
      .replace(/^#\/?/, "")
      .split("/")
      .slice(2)
      .join("/");
    return { at: "host", host: second, inside };
  }
  if (first === "apps" && second) return { at: "app", server: second };
  return { at: "home" };
}

export function address(place: Place): string {
  const part = encodeURIComponent;
  switch (place.at) {
    case "agent":
      if (place.tab === "chat") return place.chat ? `#/agents/${part(place.agent)}/chats/${part(place.chat)}` : `#/agents/${part(place.agent)}`;
      return `#/agents/${part(place.agent)}/${place.tab}`;
    case "chat":
      return `#/chats/${part(place.chat)}`;
    case "new-agent":
      return "#/agents/new";
    case "providers":
      return `#/providers/${place.tab}`;
    case "store":
      return "#/store";
    case "settings":
      return `#/settings/${place.tab}`;
    case "app":
      return `#/apps/${part(place.server)}`;
    case "host":
      return place.inside ? `#/hosts/${part(place.host)}/${place.inside}` : `#/hosts/${part(place.host)}`;
    default:
      return "#/";
  }
}

const listeners = new Set<() => void>();
let current: Place = typeof window === "undefined" ? { at: "home" } : read(window.location.hash);
let currentHash = typeof window === "undefined" ? "" : window.location.hash;

if (typeof window !== "undefined") {
  window.addEventListener("hashchange", () => {
    if (window.location.hash === currentHash) return;
    currentHash = window.location.hash;
    current = read(currentHash);
    for (const listener of listeners) listener();
  });
}

export function go(place: Place): void {
  const next = address(place);
  if (next === currentHash) return;
  currentHash = next;
  current = place;
  window.history.pushState(null, "", next);
  for (const listener of listeners) listener();
}

export function usePlace(): Place {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => current,
  );
}
