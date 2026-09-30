// The door of a Workbench served at an address: who is in, and the
// ceremonies by which a device is registered and comes in. The ceremony
// itself is the browser's; what is here carries what the host asks to the
// browser and what the device answers back to the host.

import { startAuthentication, startRegistration } from "@simplewebauthn/browser";

import { createStore } from "zustand/vanilla";

import { fetchJson } from "../http.ts";

/// What the product is called: SWEM, or the name of a product the harness
/// is built into.
export const named = createStore<{ called: string }>(() => ({ called: "SWEM" }));

export type May = "everything" | "say-what-is-due";

export type Who =
  | { by: "this-run" }
  | { by: "device"; device_id: string; name: string }
  | { by: "code" }
  | { by: "token"; token_id: string; name: string; may: May[] }
  | { by: "embedder" };

export interface DoorStanding {
  /// Where this Workbench is served; nothing when it is on this computer alone.
  at: string | null;
  name?: string;
  claimed: boolean;
  who: Who | null;
  called?: string;
}

export interface Device {
  device_id: string;
  name: string;
  added_ms: number;
  last_ms: number | null;
  last_from: string | null;
  this: boolean;
}

export interface Token {
  token_id: string;
  name: string;
  may: May[];
  made_ms: number;
  last_ms: number | null;
  last_from: string | null;
}

export interface Happened {
  at_ms: number;
  what: string;
  refused: boolean;
  who: string;
  from: string;
}

export interface AccessShown {
  at: string;
  way: "certificate" | "proxy" | "this-machine";
  apps_at: string | null;
  certificate_good_until_ms: number | null;
  devices: Device[];
  tokens: Token[];
  codes: { left: number; made: number; made_ms: number | null };
  happened: Happened[];
}

interface Begun {
  ceremony: string;
  asked: { publicKey: unknown };
}

interface CameIn {
  who: Who;
  codes: string[] | null;
}

const send = (body: unknown): RequestInit => ({ method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });

export async function doorStanding(): Promise<DoorStanding> {
  const standing = await fetchJson<DoorStanding>("/api/access");
  const called = standing.called?.trim() || "SWEM";
  if (named.getState().called !== called) named.setState({ called });
  if (typeof document !== "undefined") document.title = called === "SWEM" ? "SWEM Workbench" : called;
  return standing;
}

/// What the browser said when a person turned the ceremony down or the
/// device would not do it, in words of our own: the browser's are long
/// and name the standard.
function declined(error: unknown): Error {
  const name = error instanceof Error ? error.name : "";
  if (name === "NotAllowedError" || name === "AbortError") return new Error("Nothing was registered: the device was not asked, or it was turned down.");
  if (name === "InvalidStateError") return new Error("This device is registered here already. Sign in with it.");
  if (name === "NotSupportedError" || name === "SecurityError") return new Error("This browser cannot hold a passkey for this address.");
  return error instanceof Error ? error : new Error(String(error));
}

/// Register this device with a word that was given, and come in.
export async function register(word: string, name: string): Promise<CameIn> {
  const begun = await fetchJson<Begun>("/api/access/register/begin", send({ word, name }));
  let answered: unknown;
  try {
    answered = await startRegistration({ optionsJSON: begun.asked.publicKey as Parameters<typeof startRegistration>[0]["optionsJSON"] });
  } catch (error) {
    throw declined(error);
  }
  return fetchJson<CameIn>("/api/access/register/finish", send({ ceremony: begun.ceremony, answered }));
}

/// Come in with a device that was registered.
export async function signIn(): Promise<CameIn> {
  const begun = await fetchJson<Begun>("/api/access/sign-in/begin", send({}));
  let answered: unknown;
  try {
    answered = await startAuthentication({ optionsJSON: begun.asked.publicKey as Parameters<typeof startAuthentication>[0]["optionsJSON"] });
  } catch (error) {
    const name = error instanceof Error ? error.name : "";
    throw name === "NotAllowedError" || name === "AbortError" ? new Error("Nobody came in: the device was not asked, or it was turned down.") : declined(error);
  }
  return fetchJson<CameIn>("/api/access/sign-in/finish", send({ ceremony: begun.ceremony, answered }));
}

export const comeBack = (code: string): Promise<CameIn> => fetchJson<CameIn>("/api/access/come-back", send({ code }));
export const signOut = (): Promise<unknown> => fetchJson("/api/access/sign-out", send({}));
export const accessShown = (): Promise<AccessShown> => fetchJson<AccessShown>("/api/access/standing");
export const wordForADevice = (): Promise<{ word: string; good_for_minutes: number }> => fetchJson("/api/access/devices/word", send({}));
export const takeAway = (device: string): Promise<unknown> => fetchJson(`/api/access/devices/${encodeURIComponent(device)}`, { method: "DELETE" });
export const makeToken = (name: string, may: May[]): Promise<{ token: Token; opens: string }> => fetchJson("/api/access/tokens", send({ name, may }));
export const withdraw = (token: string): Promise<unknown> => fetchJson(`/api/access/tokens/${encodeURIComponent(token)}`, { method: "DELETE" });
export const newCodes = (): Promise<{ codes: string[] }> => fetchJson("/api/access/codes", send({}));

/// What this device is likely to be called, for a person to correct.
export function likelyName(agent: string = typeof navigator === "undefined" ? "" : navigator.userAgent): string {
  if (/iPhone/.test(agent)) return "iPhone";
  if (/iPad/.test(agent)) return "iPad";
  if (/Android/.test(agent)) return /Mobile/.test(agent) ? "Android phone" : "Android tablet";
  if (/Macintosh|Mac OS X/.test(agent)) return "Mac";
  if (/Windows/.test(agent)) return "Windows computer";
  if (/CrOS/.test(agent)) return "Chromebook";
  if (/Linux/.test(agent)) return "Linux computer";
  return "";
}

/// What a token may do, as a person reads it.
export function mayInWords(may: May[]): string {
  if (may.includes("everything")) return "May do what the page does";
  if (may.includes("say-what-is-due")) return "May say that something is due, and nothing else";
  return "May do nothing";
}
