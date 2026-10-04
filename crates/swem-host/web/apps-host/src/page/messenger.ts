// The page opened from a bot inside a messenger: the messenger puts its
// signed data on who opened it into the address's fragment, under a name
// the bot's channel told the harness (`?signed=`), and the bot says what to
// open (`?open=`, `?channel=`). The data is exchanged once, before anything
// is drawn, for a session of the Workbench's own door; from then on the
// page is the page, drawn for somebody who came through a messenger. The
// page names no messenger.

import { fetchJson } from "../http.ts";

export interface Pointed {
  channel: string;
  open: "chat" | "files" | "apps" | "question";
  question?: string;
  chat?: string;
}

const OPENS = new Set(["chat", "files", "apps", "question"]);

/// What the bot's button carried, if the page was opened from one.
export function pointedAt(search: string = typeof location === "undefined" ? "" : location.search): Pointed | null {
  const query = new URLSearchParams(search);
  const channel = query.get("channel");
  if (!channel) return null;
  const open = query.get("open") ?? "chat";
  return {
    channel,
    open: OPENS.has(open) ? (open as Pointed["open"]) : "chat",
    ...(query.get("question") ? { question: query.get("question") ?? undefined } : {}),
    ...(query.get("chat") ? { chat: query.get("chat") ?? undefined } : {}),
  };
}

/// Exchange the signed data the messenger put in the fragment for a
/// session, when there is any; the fragment is then cleared, so that the
/// page's own places can use it. Resolves to whether an exchange was made.
export async function comeInFromTheMessenger(): Promise<boolean> {
  if (typeof location === "undefined") return false;
  const pointed = pointedAt();
  const signedUnder = new URLSearchParams(location.search).get("signed");
  if (!pointed || !signedUnder) return false;
  const fragment = new URLSearchParams(location.hash.replace(/^#/, ""));
  const signed = fragment.get(signedUnder);
  if (!signed) return false;
  // Whatever happens, the fragment is not kept in the address.
  history.replaceState(null, "", `${location.pathname}${location.search}`);
  await fetchJson(`/api/access/by-channel/${encodeURIComponent(pointed.channel)}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ init_data: signed }),
  });
  return true;
}
