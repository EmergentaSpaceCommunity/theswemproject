// A chat of several: who is in it and what it is called.

import { Dialog } from "@base-ui/react/dialog";
import { useState } from "react";
import { useStore } from "zustand";

import { go } from "./place.ts";
import { Avatar } from "./who.tsx";
import { act, world } from "./world.ts";

export function NewChat({ open, onClose }: { open: boolean; onClose: () => void }) {
  const participants = useStore(world, (state) => state.participants);
  const agents = Object.values(participants)
    .filter((one) => one.kind === "agent" && !one.retired && one.profile_id)
    .sort((left, right) => left.name.localeCompare(right.name));
  const [title, setTitle] = useState("");
  const [chosen, setChosen] = useState<string[]>([]);
  const [problem, setProblem] = useState("");
  const [making, setMaking] = useState(false);
  const make = async () => {
    setMaking(true);
    setProblem("");
    try {
      const chat = await act.startChat(chosen, title.trim());
      setTitle("");
      setChosen([]);
      onClose();
      go({ at: "chat", chat: chat.chat_id });
    } catch (error) {
      setProblem((error as Error).message);
    } finally {
      setMaking(false);
    }
  };
  return (
    <Dialog.Root open={open} onOpenChange={(next) => (next ? undefined : onClose())}>
      <Dialog.Portal>
        <Dialog.Backdrop className="w-scrim" />
        <Dialog.Popup className="k-dialog w-dialog">
          <Dialog.Title className="k-heading">New chat</Dialog.Title>
          <Dialog.Description className="k-caption">
            Agents meet in a chat they are both in. Each answers when it is named with @.
          </Dialog.Description>
          <label className="k-stack w-close">
            <span className="k-caption">What it is called</span>
            <input className="k-field" value={title} maxLength={160} onChange={(event) => setTitle(event.target.value)} placeholder="Release 0.9" />
          </label>
          <fieldset className="w-fieldset">
            <legend className="k-caption">Who is in it</legend>
            {agents.map((agent) => {
              const profile = agent.profile_id ?? "";
              const inIt = chosen.includes(profile);
              return (
                <label className={`k-rail-item${inIt ? " k-active" : ""}`} key={agent.participant_id}>
                  <input
                    type="checkbox"
                    checked={inIt}
                    onChange={() => setChosen(inIt ? chosen.filter((one) => one !== profile) : [...chosen, profile])}
                  />
                  <Avatar who={agent} size="small" />
                  <span className="k-two">
                    <span className="k-name">{agent.name}</span>
                    <span className="k-mono k-muted">@{agent.handle}</span>
                  </span>
                </label>
              );
            })}
          </fieldset>
          {problem ? <div className="k-notice k-danger">{problem}</div> : null}
          <div className="k-inline w-end">
            <Dialog.Close className="k-btn k-quiet">Not now</Dialog.Close>
            <button type="button" className="k-btn k-primary" disabled={chosen.length < 2 || making} onClick={() => void make()}>
              Start the chat
            </button>
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
