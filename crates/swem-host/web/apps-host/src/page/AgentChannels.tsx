// An agent's channels: the doors into its chats, and who may write to it.
//
// A channel brings messages in and carries answers out; the chat itself
// lives here. Who may write is whoever is in a chat with the agent: you,
// the agents it shares a chat of several with, and the people its bots met
// whom you allowed.

import { useEffect, useState } from "react";

import { fetchJson } from "../http.ts";
import { AddBot, send, type ChannelShown, type ChannelsStanding } from "./Channels.tsx";
import { go } from "./place.ts";
import { useStore } from "zustand";

import { ChatSign, Plug, Prompt } from "./icons.tsx";
import { Standing } from "./standing.tsx";
import type { Participant } from "./types.ts";
import { Avatar } from "./who.tsx";
import { chatsOf, world } from "./world.ts";

export function AgentChannels({ agent, hidden }: { agent: Participant; hidden: boolean }) {
  const owner = useStore(world, (state) => state.owner);
  const together = useStore(world, (state) =>
    chatsOf(state, agent.participant_id)
      .flatMap((chat) => chat.members)
      .filter((member) => member.kind === "agent" && !member.retired && member.participant_id !== agent.participant_id)
      .map((member) => member.participant_id)
      .join(" "),
  );
  const participants = useStore(world, (state) => state.participants);
  const others = [...new Set(together.split(" ").filter(Boolean))].flatMap((id) => (participants[id] ? [participants[id]] : []));
  const [copied, setCopied] = useState(false);
  const command = `swem acp --profile ${agent.profile_id ?? ""}`;
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
    if (hidden) return;
    void read();
    const again = window.setInterval(() => void read(), 5000);
    return () => window.clearInterval(again);
  }, [hidden]);
  const bots = (standing?.channels ?? []).filter((channel) => channel.agent === agent.profile_id);
  const packages = standing?.packages ?? [];
  const allow = async (channel: ChannelShown, guest: string, may: boolean) => {
    setProblem("");
    try {
      setStanding(await fetchJson<ChannelsStanding>(`/api/channels/${encodeURIComponent(channel.id)}/guests/${encodeURIComponent(guest)}/${may ? "allow" : "forbid"}`, send("POST")));
    } catch (failure) {
      setProblem(failure instanceof Error ? failure.message : String(failure));
    }
  };
  const words = (channel: ChannelShown) => (!channel.running ? "Not running" : channel.paired ? "Running" : "Waiting for your code");
  return (
    <section className="w-page" data-agent-panel="channels" hidden={hidden} aria-label={`Channels of ${agent.name}`}>
      <section className="k-card k-stack">
        <div className="w-col w-close">
          <h2 className="k-heading">Channels</h2>
          <span className="k-caption">Doors into {agent.name}'s chats. A channel brings messages in and carries answers out; the chat itself lives here.</span>
        </div>
        <div className="k-row w-nowrap w-top">
          <ChatSign size={18} />
          <span className="k-two k-grow">
            <span className="k-name">The Workbench</span>
            <span className="k-caption">Chats with you and group chats.</span>
          </span>
          <Standing tone="ready">On</Standing>
        </div>
        <div className="k-row w-nowrap w-top">
          <Prompt size={18} />
          <span className="w-col w-close k-grow">
            <span className="k-name">Your editor</span>
            <span className="k-caption">Zed, JetBrains and others that speak the Agent Client Protocol work with {agent.name} when they are pointed at its command.</span>
            <code className="k-mono">{command}</code>
          </span>
          <button
            type="button"
            className="k-btn"
            onClick={() => {
              void navigator.clipboard
                ?.writeText(command)
                .then(() => setCopied(true))
                .catch(() => setCopied(false));
            }}
          >
            {copied ? "Copied" : "Copy the command"}
          </button>
        </div>
        {bots.map((channel) => (
          <div className="k-row w-nowrap w-top agent-bot" data-channel={channel.id} key={channel.id}>
            <Plug size={18} />
            <span className="w-col w-close k-grow">
              <span className="k-name">
                {channel.name}
                {channel.bot?.username ? <span className="k-caption k-muted"> · @{channel.bot.username}</span> : null}
              </span>
              <span className="k-caption">
                A bot in a messenger: {agent.name} answers there.
                {channel.running && !channel.paired && channel.pairing_code ? ` Say this code to the bot once: ${channel.pairing_code}.` : ""}
              </span>
              {channel.said ? <span className="k-caption k-is-danger">{channel.said}</span> : null}
            </span>
            <span className="k-inline w-tight w-nowrap">
              <button type="button" className="k-btn k-quiet" onClick={() => go({ at: "providers", tab: "channels" })}>
                Set up
              </button>
              <Standing tone={!channel.running ? "none" : channel.paired ? "ready" : "asking"}>{words(channel)}</Standing>
            </span>
          </div>
        ))}
        <div className="k-row w-nowrap w-top">
          <Plug size={18} />
          <span className="k-two k-grow">
            <span className="k-name">{bots.length > 0 ? "Another messenger bot" : "A messenger bot"}</span>
            <span className="k-caption">
              {packages.length > 0
                ? `A bot of yours in a messenger, answered by ${agent.name}. Whoever writes to it is a guest; you allow who may speak.`
                : "Nothing here can run a messenger bot: install a channel from the Store."}
            </span>
          </span>
          {packages.length > 0 ? (
            <button type="button" className="k-btn" id="agent-add-bot" onClick={() => setAdding(true)}>
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
            forAgent={agent.profile_id}
            onAdded={() => {
              setAdding(false);
              void read();
            }}
            onClose={() => setAdding(false)}
          />
        ) : null}
      </section>
      <section className="k-card k-stack">
        <div className="w-col w-close">
          <h2 className="k-heading">Who may write to {agent.name}</h2>
          <span className="k-caption">Everyone in a chat is a participant with a name: you, a guest, another agent.</span>
        </div>
        {owner ? (
          <div className="k-row w-nowrap">
            <Avatar who={owner} />
            <span className="k-two k-grow">
              <span className="k-name">{owner.name}</span>
              <span className="k-caption k-mono">@{owner.handle}</span>
            </span>
            <span className="k-caption">you · owner</span>
          </div>
        ) : null}
        {others.map((other) => (
          <div className="k-row w-nowrap" key={other.participant_id}>
            <Avatar who={other} />
            <span className="k-two k-grow">
              <span className="k-name">{other.name}</span>
              <span className="k-caption k-mono">@{other.handle}</span>
            </span>
            <span className="k-caption">agent · in group chats</span>
          </div>
        ))}
        {bots.flatMap((channel) =>
          (channel.people ?? []).map((guest) => (
            <div className="k-row w-nowrap" key={`${channel.id}:${guest.participant_id}`} data-guest={guest.participant_id} data-may-speak={guest.may_speak ? "true" : "false"}>
              <span className="k-two k-grow">
                <span className="k-name">{guest.name}</span>
                <span className="k-caption">
                  guest · met by {channel.name} · {guest.may_speak ? "may speak to the agent" : "heard, may not speak"}
                </span>
              </span>
              <button type="button" className="k-btn k-quiet" onClick={() => void allow(channel, guest.participant_id, !guest.may_speak)}>
                {guest.may_speak ? "Forbid" : "Allow"}
              </button>
            </div>
          )),
        )}
        {bots.length === 0 ? (
          <span className="k-caption">Nobody else reaches {agent.name}: a guest comes through a messenger bot, and none answers with {agent.name} yet.</span>
        ) : bots.every((channel) => (channel.people ?? []).length === 0) ? (
          <span className="k-caption">Nobody else has written to its bots yet. Whoever does is listed here, and may not speak until you allow them.</span>
        ) : null}
      </section>
    </section>
  );
}
