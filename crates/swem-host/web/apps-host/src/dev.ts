// The dev switch: whether the page shows what it carries for whoever builds
// it - route ids, the event log, the raw resource inputs - beside what it
// shows a person. Off by default; a person never asked for a connection id.
//
// It is turned on with `?dev=1` in the address (and off with `?dev=0`), kept
// in this browser like the theme, and the parameter is taken off the address
// so a link copied afterwards does not carry it. The walks that read those
// ids do not need it on: the elements stay in the page, hidden, and a hidden
// element still has a value and a text.

import { useSyncExternalStore } from "react";

const DEV_KEY = "swem.workbench.dev";

let current = readInitial();
const listeners = new Set<() => void>();

function readInitial(): boolean {
  let stored: boolean | null = null;
  try {
    const value = localStorage.getItem(DEV_KEY);
    if (value === "1") stored = true;
    if (value === "0") stored = false;
  } catch {
    // Storage is a convenience; the switch still works for this load.
  }
  try {
    const url = new URL(location.href);
    const asked = url.searchParams.get("dev");
    if (asked === "1" || asked === "0") {
      stored = asked === "1";
      url.searchParams.delete("dev");
      history.replaceState(history.state, "", url.toString());
      try {
        localStorage.setItem(DEV_KEY, asked);
      } catch {
        // As above.
      }
    }
  } catch {
    // No address to read; the stored value stands.
  }
  return stored ?? false;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function isDev(): boolean {
  return current;
}

export function setDev(on: boolean): void {
  current = on;
  try {
    localStorage.setItem(DEV_KEY, on ? "1" : "0");
  } catch {
    // As above.
  }
  for (const listener of listeners) listener();
}

export function toggleDev(): void {
  setDev(!current);
}

export function useDev(): boolean {
  return useSyncExternalStore(subscribe, isDev, isDev);
}
