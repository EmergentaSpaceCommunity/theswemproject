// Who somebody is and what they are doing, as the page says it everywhere.

import { useStore } from "zustand";

import { useSession } from "../agent/store.ts";
import type { Profile } from "../agent/session.ts";
import type { Participant } from "./types.ts";
import { doing, world, type Doing } from "./world.ts";

const TONES = new Set(["info", "success", "warning", "danger"]);

export function Avatar({ who, size }: { who: Participant | undefined; size?: "small" | "large" }) {
  const tone = who?.colour && TONES.has(who.colour) ? ` k-is-${who.colour}` : "";
  const round = who && who.kind !== "agent" ? " k-round" : "";
  const sized = size ? ` k-${size}` : "";
  const letter = (who?.name ?? "?").trim().slice(0, 1).toUpperCase() || "?";
  return (
    <span className={`k-avatar${sized}${round}${tone}`} aria-hidden="true">
      {letter}
    </span>
  );
}

const WORDS: Record<Doing, string> = {
  working: "Working",
  asking: "Waiting for you",
  waiting: "Next in line",
  ready: "Ready",
};
const DOTS: Record<Doing, string> = {
  working: "k-busy",
  asking: "k-asking",
  waiting: "k-busy",
  ready: "k-ready",
};

export function useDoing(agentId: string, chatId?: string): Doing {
  return useStore(world, (state) => doing(state, agentId, chatId));
}

export function StateDot({ what }: { what: Doing }) {
  return <span className={`k-dot k-small ${DOTS[what]}`} title={WORDS[what]} />;
}

export function StateWord({ what }: { what: Doing }) {
  return (
    <span className="k-status">
      <StateDot what={what} />
      <span>{WORDS[what]}</span>
    </span>
  );
}

/// What an agent stands on and where it lives, in the words a person knows
/// them by: the engine's name, and the host's.
export function useStanding(agent: Participant | undefined): { profile: Profile | null; engine: string; host: string } {
  const { profiles, onboarding, environments } = useSession();
  const profile = profiles.find((one) => one.profile_id === agent?.profile_id) ?? null;
  const engine = onboarding?.agents.find((one) => one.agent_id === profile?.agent_id)?.name ?? profile?.agent_id ?? "";
  const host = environments.find((one) => one.environment_profile_id === profile?.environment_profile_id)?.name ?? "";
  return { profile, engine, host };
}

export const names = (members: Participant[], owner: Participant | null): string =>
  members.map((member) => (member.participant_id === owner?.participant_id ? "You" : member.name)).join(", ");

export function when(at?: number): string {
  if (!at) return "";
  const then = new Date(at);
  const now = new Date();
  const sameDay = then.toDateString() === now.toDateString();
  const clock = then.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return sameDay ? clock : `${then.toLocaleDateString([], { day: "numeric", month: "short" })}, ${clock}`;
}
