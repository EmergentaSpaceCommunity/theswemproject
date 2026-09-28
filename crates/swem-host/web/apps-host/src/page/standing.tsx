// How something stands, and who stands on it, as Providers says it.

import { useStore } from "zustand";

import { Avatar } from "./who.tsx";
import { world } from "./world.ts";

/// The agents that stand on something, by their faces.
export function UsedBy({ profiles }: { profiles: string[] }) {
  const participants = useStore(world, (state) => state.participants);
  const agents = Object.values(participants).filter((one) => one.kind === "agent" && one.profile_id && profiles.includes(one.profile_id));
  if (agents.length === 0) return <span className="k-caption">No agents yet</span>;
  return (
    <span className="k-inline w-tight w-nowrap" title={agents.map((agent) => agent.name).join(", ")}>
      {agents.map((agent) => (
        <Avatar who={agent} size="small" key={agent.participant_id} />
      ))}
    </span>
  );
}

export function Standing({ tone, children }: { tone: "ready" | "asking" | "none"; children: string }) {
  return (
    <span className="k-status">
      <span className={`k-dot k-small${tone === "none" ? "" : ` k-${tone}`}`} />
      <span>{children}</span>
    </span>
  );
}
