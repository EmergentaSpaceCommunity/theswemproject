// The one SessionStore the page runs on, and the hook that reads it.
//
// The store is created once, outside React, because its long-poll loops
// outlive any component: a rail that unmounts while a turn is running must
// not take the turn's events with it.

import { at } from "../base.ts";
import { useSyncExternalStore } from "react";

import { SessionStore, type SessionState } from "./session.ts";

function browserStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export const sessionStore = new SessionStore((input, init) => fetch(typeof input === "string" ? at(input) : input, init), browserStorage());

export function useSession(): SessionState {
  return useSyncExternalStore(sessionStore.subscribe, sessionStore.getSnapshot);
}
