// An agent's channels: the doors into its chats, and who may write to it.
//
// A channel brings messages in and carries answers out; the chat itself
// lives here. Who may write is whoever is in a chat with the agent: you,
// and the agents it shares a chat of several with.

import { useState } from "react";
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
        <div className="k-row w-nowrap w-top">
          <Plug size={18} />
          <span className="k-two k-grow">
            <span className="k-name">A messenger bot</span>
            <span className="k-caption">Links a messenger chat to a chat here. Its people become guests with names you allow.</span>
          </span>
          <Standing tone="none">Cannot be connected yet</Standing>
        </div>
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
        <span className="k-caption">Nobody else reaches {agent.name}: a guest comes through a messenger, and none is connected.</span>
      </section>
    </section>
  );
}
