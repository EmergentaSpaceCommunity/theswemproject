// Beside a chat of several: who is in it and what each is doing, bringing
// an agent in and taking one out, and the chat's two rules.

import { Dialog } from "@base-ui/react/dialog";
import { useEffect, useState } from "react";
import { useStore } from "zustand";

import { Plus } from "./icons.tsx";
import type { Chat, Participant } from "./types.ts";
import { Avatar, StateWord, useDoing } from "./who.tsx";
import { act, world } from "./world.ts";

function Member({ chat, member, owner }: { chat: Chat; member: Participant; owner: Participant | null }) {
  const what = useDoing(member.participant_id, chat.chat_id);
  const [problem, setProblem] = useState("");
  const you = member.participant_id === owner?.participant_id;
  const began = member.participant_id === chat.created_by;
  return (
    <div className="k-row w-nowrap">
      <Avatar who={member} size="small" />
      <span className="k-two k-grow">
        <span className="k-name">{member.name}</span>
        <span className="k-mono k-muted">@{member.handle}</span>
      </span>
      <span className="w-col w-close w-end-items">
        <span className="k-caption">{you ? "you" : member.kind}</span>
        {member.kind === "agent" ? <StateWord what={what} /> : null}
        {problem ? <span className="k-caption k-is-danger">{problem}</span> : null}
      </span>
      {(member.kind === "agent" || member.kind === "guest") && !began ? (
        <button
          type="button"
          className="k-btn k-quiet"
          aria-label={`Take ${member.name} out of the chat`}
          onClick={() => {
            setProblem("");
            act.takeOut(chat.chat_id, member.participant_id).catch((error: Error) => setProblem(error.message));
          }}
        >
          Take out
        </button>
      ) : null}
    </div>
  );
}

/// Bring in an agent that is not in the chat yet.
export function AddSomeone({ chat, open, onClose }: { chat: Chat; open: boolean; onClose: () => void }) {
  const participants = useStore(world, (state) => state.participants);
  const [problem, setProblem] = useState("");
  // A guest who wrote to a bot a minute ago is not known to the page yet.
  useEffect(() => {
    if (open) void act.people().catch(() => undefined);
  }, [open]);
  const inIt = new Set(chat.members.map((member) => member.participant_id));
  const others = Object.values(participants)
    .filter((one) => one.kind === "agent" && !one.retired && one.profile_id && !inIt.has(one.participant_id))
    .sort((left, right) => left.name.localeCompare(right.name));
  // Guests: people who wrote to a bot of yours and were told to wait.
  const guests = Object.values(participants)
    .filter((one) => one.kind === "guest" && !one.retired && !inIt.has(one.participant_id))
    .sort((left, right) => left.name.localeCompare(right.name));
  const bring = (agent: Participant) => {
    setProblem("");
    act
      .bring(chat.chat_id, agent.profile_id ?? "")
      .then(onClose)
      .catch((error: Error) => setProblem(error.message));
  };
  const letIn = (guest: Participant) => {
    setProblem("");
    act
      .letIn(chat.chat_id, guest.participant_id)
      .then(onClose)
      .catch((error: Error) => setProblem(error.message));
  };
  return (
    <Dialog.Root open={open} onOpenChange={(next) => (next ? undefined : onClose())}>
      <Dialog.Portal>
        <Dialog.Backdrop className="w-scrim" />
        <Dialog.Popup className="k-dialog w-dialog">
          <Dialog.Title className="k-heading">Add someone</Dialog.Title>
          <Dialog.Description className="k-caption">An agent that is brought in reads what is said from then on, and answers when it is named.</Dialog.Description>
          <div className="k-rail-group">
            {others.map((agent) => (
              <button type="button" className="k-rail-item" key={agent.participant_id} onClick={() => bring(agent)}>
                <Avatar who={agent} size="small" />
                <span className="k-two">
                  <span className="k-name">{agent.name}</span>
                  <span className="k-mono k-muted">@{agent.handle}</span>
                </span>
              </button>
            ))}
            {others.length === 0 ? <span className="k-caption">Every agent you have is in this chat.</span> : null}
          </div>
          {guests.length > 0 ? (
            <div className="k-rail-group">
              <span className="k-eyebrow">Guests, from a messenger</span>
              {guests.map((guest) => (
                <button type="button" className="k-rail-item" data-guest={guest.participant_id} key={guest.participant_id} onClick={() => letIn(guest)}>
                  <Avatar who={guest} size="small" />
                  <span className="k-two">
                    <span className="k-name">{guest.name}</span>
                    <span className="k-caption">Let in: what they write to the bot is said here, and what is said here reaches them.</span>
                  </span>
                </button>
              ))}
            </div>
          ) : null}
          {problem ? <div className="k-notice k-danger">{problem}</div> : null}
          <div className="k-inline w-end">
            <Dialog.Close className="k-btn k-quiet">Not now</Dialog.Close>
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

export function Members({ chat, onAdd }: { chat: Chat; onAdd: () => void }) {
  const owner = useStore(world, (state) => state.owner);
  const [problem, setProblem] = useState("");
  const change = (what: { answer_rule?: string; reply_limit?: number }) => {
    setProblem("");
    act.change(chat.chat_id, what).catch((error: Error) => setProblem(error.message));
  };
  const members = chat.members.filter((member) => member.kind !== "schedule" && !member.retired);
  return (
    <aside className="w-side w-right" aria-label="About this chat">
      <div className="k-rail-group">
        <span className="k-eyebrow">In this chat</span>
        <div>
          {members.map((member) => (
            <Member chat={chat} member={member} owner={owner} key={member.participant_id} />
          ))}
        </div>
        <button type="button" className="k-btn" onClick={onAdd}>
          <Plus size={15} />
          <span>Add someone</span>
        </button>
      </div>
      <div className="k-rail-group">
        <span className="k-eyebrow">Rules</span>
        <label className="k-stack w-close">
          <span className="k-caption">Agents answer</span>
          <select className="k-field" value={chat.answer_rule} onChange={(event) => change({ answer_rule: event.target.value })}>
            <option value="named">When they are named</option>
            <option value="always">Whatever is said</option>
          </select>
        </label>
        <label className="k-stack w-close">
          <span className="k-caption">Replies of agents to each other in a row</span>
          <select className="k-field" value={String(chat.reply_limit)} onChange={(event) => change({ reply_limit: Number(event.target.value) })}>
            {[1, 2, 3, 4, 6, 8, 12, 20].concat([1, 2, 3, 4, 6, 8, 12, 20].includes(chat.reply_limit) ? [] : [chat.reply_limit]).sort((left, right) => left - right).map((limit) => (
              <option value={String(limit)} key={limit}>
                {limit}, then wait for a person
              </option>
            ))}
          </select>
        </label>
        {problem ? <span className="k-caption k-is-danger">{problem}</span> : null}
      </div>
    </aside>
  );
}
