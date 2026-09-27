// What only a live session of the engine has: what it lets a person choose,
// and the Apps its servers bring. A page asks for the session of an agent
// in a chat; the engine is started only when the page says to.

import { useEffect, useState } from "react";

import { readSessionOptions, type SessionOptions } from "../agent/session.ts";
import { fetchJson } from "../http.ts";

const part = encodeURIComponent;
const post = (body: unknown): RequestInit => ({
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify(body),
});

export async function sessionOf(chat: string, agent: string, open: boolean): Promise<string | null> {
  const answer = await fetchJson<{ connection_id: string | null }>(
    `/api/chats/${part(chat)}/agents/${part(agent)}/session`,
    open ? post({}) : undefined,
  );
  return answer.connection_id;
}

/// What the session of an agent in a chat lets a person choose, read again
/// whenever `changed` moves. Nothing, while the session is not live.
export function useOffers(chat: string, agent: string | undefined, changed: number): {
  connection: string | null;
  offers: SessionOptions | null;
  choose: (option: string, value: string) => Promise<void>;
  mode: (mode: string) => Promise<void>;
} {
  const [connection, setConnection] = useState<string | null>(null);
  const [offers, setOffers] = useState<SessionOptions | null>(null);
  const [asked, setAsked] = useState(0);
  useEffect(() => {
    let left = false;
    if (!agent) return undefined;
    void (async () => {
      try {
        const live = await sessionOf(chat, agent, false);
        if (left) return;
        setConnection(live);
        if (!live) {
          setOffers(null);
          return;
        }
        const wire = await fetchJson<Parameters<typeof readSessionOptions>[0]>(`/api/connections/${part(live)}/options`);
        if (!left) setOffers(readSessionOptions(wire));
      } catch {
        if (!left) setOffers(null);
      }
    })();
    return () => {
      left = true;
    };
  }, [chat, agent, changed, asked]);
  return {
    connection,
    offers,
    choose: async (option, value) => {
      if (!connection) return;
      await fetchJson(`/api/connections/${part(connection)}/options`, post({ config_id: option, value }));
      setAsked((count) => count + 1);
    },
    mode: async (mode) => {
      if (!connection) return;
      await fetchJson(`/api/connections/${part(connection)}/mode`, post({ mode_id: mode }));
      setAsked((count) => count + 1);
    },
  };
}
