// Channels: the doors into chats. Two are built in, this page and an
// editor that hosts agents; a messenger is said and cannot be added yet.

import { useSession } from "../agent/store.ts";
import { ChatSign, Plug, Prompt } from "./icons.tsx";
import { Standing } from "./standing.tsx";

export function Channels() {
  const { profiles } = useSession();
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
      <div className="k-row w-nowrap w-top">
        <Plug size={18} />
        <span className="k-two k-grow">
          <span className="k-name">A messenger</span>
          <span className="k-caption">A bot in a messenger, linked to a chat here, with the people who write there as guests of the chat.</span>
        </span>
        <Standing tone="none">Cannot be added yet</Standing>
      </div>
    </section>
  );
}
