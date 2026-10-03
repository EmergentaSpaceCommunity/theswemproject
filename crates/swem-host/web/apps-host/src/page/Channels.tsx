// Channels: the doors into chats. Two are built in, this page and an
// editor that hosts agents; a messenger is a bot a person adds, run by a
// channel package, and whoever writes to it is a guest of the chat - or the
// owner, once they said the code.

import { Dialog } from "@base-ui/react/dialog";
import { useEffect, useState } from "react";

import { useSession } from "../agent/store.ts";
import { fetchJson } from "../http.ts";
import { ChatSign, Plug, Prompt } from "./icons.tsx";
import { Standing } from "./standing.tsx";

export type GuestPolicy = "nobody" | "by_invitation" | "anyone";

export interface ChannelShown {
  id: string;
  name: string;
  package?: string;
  program?: string;
  agent?: string;
  guests: GuestPolicy;
  settings: { api_root?: string; door?: string };
  pairing_code?: string;
  bot?: { id: string; username: string; name: string };
  running: boolean;
  keyed: boolean;
  paired: boolean;
  said?: string;
}

export interface ChannelPackage {
  id: string;
  name: string;
  version?: string;
  bundled: boolean;
}

interface ChannelsStanding {
  channels: ChannelShown[];
  packages: ChannelPackage[];
}

const send = (method: string, body?: unknown): RequestInit => ({
  method,
  headers: { "content-type": "application/json" },
  body: body === undefined ? undefined : JSON.stringify(body),
});

const GUESTS: { id: GuestPolicy; words: string }[] = [
  { id: "nobody", words: "Nobody: the bot answers you only" },
  { id: "by_invitation", words: "By invitation: whom you join to a chat" },
  { id: "anyone", words: "Anyone: a chat of their own with the agent" },
];

function AddBot({ packages, onAdded, onClose }: { packages: ChannelPackage[]; onAdded: (shown: ChannelShown) => void; onClose: () => void }) {
  const { profiles } = useSession();
  const [name, setName] = useState("");
  const [pkg, setPkg] = useState(packages[0]?.id ?? "");
  const [key, setKey] = useState("");
  const [agent, setAgent] = useState(profiles[0]?.profile_id ?? "");
  const [guests, setGuests] = useState<GuestPolicy>("nobody");
  const [apiRoot, setApiRoot] = useState("");
  const [more, setMore] = useState(false);
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const add = async () => {
    setBusy(true);
    setProblem("");
    try {
      const shown = await fetchJson<ChannelShown>(
        "/api/channels",
        send("POST", {
          name: name.trim(),
          package: pkg,
          key: key.trim(),
          agent: agent || null,
          guests,
          settings: apiRoot.trim() ? { api_root: apiRoot.trim() } : {},
        }),
      );
      onAdded(shown);
    } catch (failure) {
      setProblem(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog.Root open onOpenChange={(next) => (next ? undefined : onClose())}>
      <Dialog.Portal>
        <Dialog.Backdrop className="w-scrim" />
        <Dialog.Popup className="k-dialog w-dialog">
          <Dialog.Title className="k-heading">Add a bot</Dialog.Title>
          <Dialog.Description className="k-caption">
            A bot you made in the messenger, with the token it gave you. The token is kept with your keys and never shown again. You will be given a code to say to the bot once, so it knows you.
          </Dialog.Description>
          <form
            className="k-stack"
            onSubmit={(event) => {
              event.preventDefault();
              void add();
            }}
          >
            <label className="k-stack w-close">
              <span className="k-caption">What you call it</span>
              <input id="channel-name" className="k-field" value={name} onChange={(event) => setName(event.target.value)} placeholder="My Telegram bot" maxLength={60} required autoFocus />
            </label>
            <label className="k-stack w-close">
              <span className="k-caption">Messenger</span>
              <select id="channel-package" className="k-field" value={pkg} onChange={(event) => setPkg(event.target.value)}>
                {packages.map((one) => (
                  <option value={one.id} key={one.id}>
                    {one.name}
                    {one.bundled ? " (came with this product)" : one.version ? ` ${one.version}` : ""}
                  </option>
                ))}
              </select>
            </label>
            <label className="k-stack w-close">
              <span className="k-caption">The bot's token</span>
              <input id="channel-key" className="k-field" type="password" autoComplete="off" spellCheck={false} value={key} onChange={(event) => setKey(event.target.value)} required />
            </label>
            <label className="k-stack w-close">
              <span className="k-caption">Who answers there</span>
              <select id="channel-agent" className="k-field" value={agent} onChange={(event) => setAgent(event.target.value)}>
                {profiles.map((profile) => (
                  <option value={profile.profile_id} key={profile.profile_id}>
                    {profile.profile_id}
                  </option>
                ))}
              </select>
            </label>
            <label className="k-stack w-close">
              <span className="k-caption">Who else may write to it</span>
              <select id="channel-guests" className="k-field" value={guests} onChange={(event) => setGuests(event.target.value as GuestPolicy)}>
                {GUESTS.map((one) => (
                  <option value={one.id} key={one.id}>
                    {one.words}
                  </option>
                ))}
              </select>
            </label>
            {more ? (
              <label className="k-stack w-close">
                <span className="k-caption">Bot API address, when not the messenger's own (a local Bot API server)</span>
                <input id="channel-api-root" className="k-field k-mono" value={apiRoot} onChange={(event) => setApiRoot(event.target.value)} placeholder="http://127.0.0.1:8081" spellCheck={false} />
              </label>
            ) : (
              <button type="button" className="k-btn k-quiet w-self-start" id="channel-more" onClick={() => setMore(true)}>
                More
              </button>
            )}
            {problem ? <div className="k-notice k-danger">{problem}</div> : null}
            <div className="k-inline w-end">
              <Dialog.Close className="k-btn k-quiet" type="button">
                Not now
              </Dialog.Close>
              <button id="channel-add" type="submit" className="k-btn k-primary" disabled={busy || !name.trim() || !key.trim() || !pkg}>
                Add
              </button>
            </div>
          </form>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function tone(channel: ChannelShown): "ready" | "asking" | "none" {
  if (!channel.running) return "none";
  return channel.paired ? "ready" : "asking";
}

function words(channel: ChannelShown): string {
  if (!channel.running) return "Not running";
  return channel.paired ? "Running" : "Waiting for your code";
}

export function Channels() {
  const { profiles } = useSession();
  const [standing, setStanding] = useState<ChannelsStanding | null>(null);
  const [adding, setAdding] = useState(false);
  const [problem, setProblem] = useState("");
  const read = async () => {
    try {
      setStanding(await fetchJson<ChannelsStanding>("/api/channels"));
    } catch (failure) {
      setProblem(failure instanceof Error ? failure.message : String(failure));
    }
  };
  useEffect(() => {
    void read();
    // Pairing happens on the messenger's side; the page follows.
    const again = window.setInterval(() => void read(), 5000);
    return () => window.clearInterval(again);
  }, []);
  const remove = async (channel: ChannelShown) => {
    try {
      await fetchJson(`/api/channels/${encodeURIComponent(channel.id)}`, send("DELETE"));
      await read();
    } catch (failure) {
      setProblem(failure instanceof Error ? failure.message : String(failure));
    }
  };
  const start = async (channel: ChannelShown) => {
    try {
      setStanding(await fetchJson<ChannelsStanding>(`/api/channels/${encodeURIComponent(channel.id)}/start`, send("POST")));
    } catch (failure) {
      setProblem(failure instanceof Error ? failure.message : String(failure));
    }
  };
  const packages = standing?.packages ?? [];
  return (
    <section className="k-card k-stack">
      <div className="w-col w-close">
        <h2 className="k-heading">Channels</h2>
        <span className="k-caption">A channel is a door into chats from outside. Whatever comes through one is said in a chat, by whoever said it.</span>
      </div>
      <div className="k-row w-nowrap w-top">
        <ChatSign size={18} />
        <span className="k-two k-grow">
          <span className="k-name">The Workbench</span>
          <span className="k-caption">Built in. This page, for you.</span>
        </span>
        <Standing tone="ready">Open</Standing>
      </div>
      <div className="k-row w-nowrap w-top">
        <Prompt size={18} />
        <span className="w-col w-close k-grow">
          <span className="k-name">Your editor</span>
          <span className="k-caption">Built in. An editor that hosts agents works with an agent of yours, in the same folder, when it is pointed at its command.</span>
          {profiles.map((profile) => (
            <code className="k-mono" key={profile.profile_id}>
              swem acp --profile {profile.profile_id}
            </code>
          ))}
        </span>
        <Standing tone="ready">Open</Standing>
      </div>
      {(standing?.channels ?? []).map((channel) => (
        <div className="k-row w-nowrap w-top channel-row" data-channel={channel.id} data-paired={channel.paired ? "true" : "false"} data-running={channel.running ? "true" : "false"} key={channel.id}>
          <Plug size={18} />
          <span className="w-col w-close k-grow">
            <span className="k-name">
              {channel.name}
              {channel.bot?.username ? <span className="k-caption k-muted"> · @{channel.bot.username}</span> : null}
            </span>
            <span className="k-caption">
              {channel.agent ? `${channel.agent} answers` : "No agent answers yet"} · guests: {GUESTS.find((one) => one.id === channel.guests)?.words.split(":")[0].toLowerCase() ?? channel.guests}
            </span>
            {channel.running && !channel.paired && channel.pairing_code ? (
              <span className="k-caption">
                Write to the bot and say this code once: <code className="k-mono channel-code">{channel.pairing_code}</code>
              </span>
            ) : null}
            {channel.said ? <span className="k-caption k-is-danger">{channel.said}</span> : null}
          </span>
          <span className="k-inline w-tight w-nowrap">
            {channel.running ? null : (
              <button className="k-btn k-quiet" onClick={() => void start(channel)}>
                Start
              </button>
            )}
            <button className="k-btn k-quiet channel-remove" onClick={() => void remove(channel)}>
              Remove
            </button>
            <Standing tone={tone(channel)}>{words(channel)}</Standing>
          </span>
        </div>
      ))}
      <div className="k-row w-nowrap w-top">
        <Plug size={18} />
        <span className="k-two k-grow">
          <span className="k-name">A messenger</span>
          <span className="k-caption">
            {packages.length > 0
              ? "A bot in a messenger, linked to a chat here. You are known there by a code; others are guests, as you allow."
              : "A bot in a messenger. Nothing here can run one yet: install a channel from the Store."}
          </span>
        </span>
        {packages.length > 0 ? (
          <button id="channel-add-bot" className="k-btn k-primary" onClick={() => setAdding(true)}>
            Add a bot
          </button>
        ) : (
          <Standing tone="none">None installed</Standing>
        )}
      </div>
      {problem ? <div className="k-notice k-danger">{problem}</div> : null}
      {adding ? (
        <AddBot
          packages={packages}
          onAdded={() => {
            setAdding(false);
            void read();
          }}
          onClose={() => setAdding(false)}
        />
      ) : null}
    </section>
  );
}
