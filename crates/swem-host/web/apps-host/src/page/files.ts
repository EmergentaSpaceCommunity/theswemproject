// An agent's files as the host serves them: the folder it works in as a
// tree, a file as it is opened and saved.

import { fetchJson } from "../http.ts";

export interface Entry {
  name: string;
  kind: "file" | "folder" | "link" | "other";
  byte_length: number;
  modified_ms?: number | null;
}

/// A file as it was opened. What is not text has no text.
export interface Opened {
  path: string;
  text?: string | null;
  sha256: string;
  byte_length: number;
}

export interface Saved {
  saved: boolean;
  sha256?: string | null;
  said?: string;
}

const part = encodeURIComponent;
const send = (method: string, body: unknown): RequestInit => ({ method, headers: { "content-type": "application/json" }, body: JSON.stringify(body) });

/// The place of a name in a folder; the folder the agent works in is "".
export const within = (folder: string, name: string): string => (folder === "" ? name : `${folder}/${name}`);
/// The folder a place is in.
export const folderOf = (place: string): string => (place.includes("/") ? place.slice(0, place.lastIndexOf("/")) : "");
/// The last name of a place.
export const nameOf = (place: string): string => place.slice(place.lastIndexOf("/") + 1);

/// Why a name cannot be given to a file, in words; nothing when it can.
export function whyNotNamed(name: string): string {
  const given = name.trim();
  if (given === "") return "It needs a name.";
  if (given === "." || given === "..") return "That is a place, not a name.";
  if (/[/\\]/.test(given)) return "A name holds no slash.";
  if (given.length > 255) return "That name is too long.";
  return "";
}

/// How long ago something was, as a person says it.
export function ago(ms: number, now: number = Date.now()): string {
  const minutes = Math.floor(Math.max(0, now - ms) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return minutes === 1 ? "a minute ago" : `${minutes} minutes ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return hours === 1 ? "an hour ago" : `${hours} hours ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return days === 1 ? "yesterday" : `${days} days ago`;
  return new Date(ms).toLocaleDateString([], { day: "numeric", month: "short", year: "numeric" });
}

/// What a size reads as.
export function sized(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/// The language a file is written in, by its name; nothing when the page
/// does not colour it.
export function languageOf(place: string): string | null {
  const name = nameOf(place).toLowerCase();
  const ending = name.includes(".") ? name.slice(name.lastIndexOf(".") + 1) : "";
  const known: Record<string, string> = {
    js: "javascript", mjs: "javascript", cjs: "javascript", jsx: "jsx", ts: "typescript", mts: "typescript", tsx: "tsx",
    json: "json", md: "markdown", markdown: "markdown", py: "python", rs: "rust", html: "html", htm: "html",
    css: "css", yml: "yaml", yaml: "yaml",
  };
  return known[ending] ?? null;
}

/// Whether a file is a picture the page can show.
export const isPicture = (place: string): boolean => /\.(png|jpe?g|svg)$/i.test(nameOf(place));

export const asItIs = (profile: string, place: string): string => `/api/profiles/${part(profile)}/file?path=${part(place)}&as=it-is`;

export const files = {
  async list(profile: string, folder: string): Promise<Entry[]> {
    const answer = await fetchJson<{ entries: Entry[] }>(`/api/profiles/${part(profile)}/tree?dir=${part(folder)}`);
    return answer.entries;
  },
  open(profile: string, place: string): Promise<Opened> {
    return fetchJson<Opened>(`/api/profiles/${part(profile)}/file?path=${part(place)}`);
  },
  /// Save what was written. `was` is what was opened; `over` is the
  /// person choosing theirs over what is there.
  save(profile: string, place: string, text: string, was: string | null, over = false): Promise<Saved> {
    return fetchJson<Saved>(`/api/profiles/${part(profile)}/file`, send("PUT", { path: place, text, was, over }));
  },
  /// Bring a file from the person's own machine. It does not replace
  /// what is there unless `over` says so.
  async bring(profile: string, place: string, file: Blob, over = false): Promise<Saved> {
    const bytes = new Uint8Array(await file.arrayBuffer());
    let binary = "";
    for (let at = 0; at < bytes.length; at += 0x8000) binary += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
    return fetchJson<Saved>(`/api/profiles/${part(profile)}/file`, send("PUT", { path: place, bytes: btoa(binary), was: null, over }));
  },
  async makeFolder(profile: string, place: string): Promise<void> {
    await fetchJson(`/api/profiles/${part(profile)}/tree`, send("POST", { do: "make_folder", path: place }));
  },
  async rename(profile: string, from: string, to: string): Promise<void> {
    await fetchJson(`/api/profiles/${part(profile)}/tree`, send("POST", { do: "rename", from, to }));
  },
  async remove(profile: string, place: string, withAll: boolean): Promise<void> {
    await fetchJson(`/api/profiles/${part(profile)}/tree`, send("POST", { do: "remove", path: place, with_all: withAll }));
  },
};
