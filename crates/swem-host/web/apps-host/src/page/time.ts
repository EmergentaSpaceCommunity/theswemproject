// Schedules: messages that arrive on time, as the host keeps them.

import { createStore } from "zustand/vanilla";

import { fetchJson } from "../http.ts";

export type When = { kind: "every"; minutes: number } | { kind: "cron"; line: string; zone: string } | { kind: "once"; at_ms: number };

export interface Schedule {
  schedule_id: string;
  agent_id: string;
  made_by: string;
  made_by_name: string;
  chat_id?: string | null;
  chat_title?: string | null;
  say: string;
  when: When;
  when_in_words: string;
  enabled: boolean;
  next_due_ms?: number | null;
}

export interface Run {
  schedule_id: string;
  say: string;
  due_ms: number;
  claimed_ms: number;
  late: boolean;
  chat_id?: string | null;
  ended_ms?: number | null;
  state: "running" | "answered" | "failed" | "stopped" | "skipped";
  note?: string | null;
}

/// One look of a keeper that starts SWEM, looks and leaves.
export interface KeeperLook {
  looked_ms: number;
  said: number;
  kept_elsewhere: boolean;
  said_last_ms?: number | null;
}

/// One who can keep time, as it stands now.
export interface KeeperShown {
  id: "swem" | "system" | "outside";
  available: boolean;
  on: boolean;
  default: boolean;
  used_by: string[];
  called?: string;
  last_look?: KeeperLook | null;
  said?: string;
}

export interface Keeper {
  keeping: boolean;
  zone: string;
  used_by: string[];
  schedules: number;
  keepers: KeeperShown[];
  /// Who was chosen for which agent, by its profile.
  chosen: Record<string, string>;
}

/// What a keeper is called on the page.
export const KEEPER_NAMES: Record<KeeperShown["id"], string> = {
  swem: "This SWEM",
  system: "The system's own scheduler",
  outside: "An outside scheduler",
};

/// Who keeps an agent's time: what was chosen for it, or what agents have.
export function keeperOf(keeper: Keeper | null, profile: string | null | undefined): KeeperShown | null {
  if (!keeper) return null;
  const chosen = profile ? keeper.chosen[profile] : undefined;
  return keeper.keepers.find((one) => (chosen ? one.id === chosen : one.default)) ?? null;
}

interface Time {
  /// Per agent, what it is told on time and how the last runs went.
  of: Record<string, { schedules: Schedule[]; runs: Run[] }>;
  keeper: Keeper | null;
}

export const time = createStore<Time>(() => ({ of: {}, keeper: null }));

const part = encodeURIComponent;
const send = (method: string, body?: unknown): RequestInit => ({
  method,
  ...(body === undefined ? {} : { headers: { "content-type": "application/json" }, body: JSON.stringify(body) }),
});

export const timing = {
  async read(agent: string): Promise<void> {
    const answer = await fetchJson<{ schedules: Schedule[]; runs: Run[] }>(`/api/schedules?agent=${part(agent)}`);
    time.setState((known) => ({ of: { ...known.of, [agent]: answer } }));
  },
  async keeper(): Promise<void> {
    time.setState({ keeper: await fetchJson<Keeper>("/api/time") });
  },
  /// Give the system's scheduler the job of starting SWEM to look.
  async turnSystemOn(makeDefault: boolean): Promise<void> {
    time.setState({ keeper: await fetchJson<Keeper>("/api/time/keepers/system/on", send("POST", { make_default: makeDefault })) });
  },
  async turnSystemOff(): Promise<void> {
    time.setState({ keeper: await fetchJson<Keeper>("/api/time/keepers/system/off", send("POST")) });
  },
  /// Agents nothing was chosen for have this keeper.
  async makeDefault(keeper: string): Promise<void> {
    time.setState({ keeper: await fetchJson<Keeper>(`/api/time/keepers/${part(keeper)}/default`, send("POST")) });
  },
  /// Who keeps an agent's time; nothing lets it have what agents have.
  async choose(profile: string, keeper: string | null): Promise<void> {
    time.setState({ keeper: await fetchJson<Keeper>(`/api/time/agents/${part(profile)}`, send("PUT", { keeper })) });
  },
  async make(agent: string, say: string, when: When, chat: string | null): Promise<void> {
    await fetchJson("/api/schedules", send("POST", { agent_id: agent, say, when, chat_id: chat }));
    await timing.read(agent);
  },
  async turn(agent: string, schedule: string, on: boolean): Promise<void> {
    await fetchJson(`/api/schedules/${part(schedule)}`, send("PATCH", { enabled: on }));
    await timing.read(agent);
  },
  async forget(agent: string, schedule: string): Promise<void> {
    await fetchJson(`/api/schedules/${part(schedule)}`, send("DELETE"));
    await timing.read(agent);
  },
};

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/// How long until a moment, as a person says it: "in 12 min", "in 7 h".
export function until(ms: number, now: number = Date.now()): string {
  const left = ms - now;
  if (left <= 0) return "now";
  if (left < MINUTE) return "in under a minute";
  if (left < HOUR) return `in ${Math.round(left / MINUTE)} min`;
  if (left < 2 * DAY) return `in ${Math.round(left / HOUR)} h`;
  return `in ${Math.round(left / DAY)} days`;
}

const clock = (ms: number): string => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

/// The day and time of a moment: "today 06:00", "yesterday 06:00", "27 Sep 06:00".
export function moment(ms: number, now: number = Date.now()): string {
  const day = (at: number) => new Date(at).toDateString();
  if (day(ms) === day(now)) return `today ${clock(ms)}`;
  if (day(ms) === day(now - DAY)) return `yesterday ${clock(ms)}`;
  return `${new Date(ms).toLocaleDateString([], { day: "numeric", month: "short" })} ${clock(ms)}`;
}

/// How long something took, as a person says it.
export function took(ms: number): string {
  if (ms < MINUTE) return `${Math.max(1, Math.round(ms / 1000))} s`;
  if (ms < HOUR) return `${Math.round(ms / MINUTE)} min`;
  return `${Math.round(ms / HOUR)} h`;
}

/// What there is to say about a run beside its state.
export function ran(run: Run, now: number = Date.now()): string {
  const said = [moment(run.due_ms, now)];
  if (run.state === "skipped") return [...said, run.note ?? "not said"].join(" · ");
  if (run.late) said.push(`nobody kept time then, said at ${clock(run.claimed_ms)}`);
  if (run.ended_ms) said.push(took(run.ended_ms - run.claimed_ms));
  if (run.note) said.push(run.note);
  return said.join(" · ");
}

export const HOW_A_RUN_ENDED: Record<Run["state"], { words: string; tone: "ready" | "asking" | "busy" | "none" }> = {
  running: { words: "Answering", tone: "busy" },
  answered: { words: "Answered", tone: "ready" },
  failed: { words: "Could not answer", tone: "asking" },
  stopped: { words: "Stopped", tone: "none" },
  skipped: { words: "Passed over", tone: "none" },
};

/// What a person chose in the form, as the moment the host keeps.
export interface Chosen {
  how: "every" | "daily" | "weekly" | "once" | "cron";
  every: number;
  unit: "minutes" | "hours" | "days";
  at: string;
  day: string;
  once: string;
  line: string;
}

export function whenOf(chosen: Chosen, zone: string): When {
  const [hour, minute] = (chosen.at || "09:00").split(":").map((one) => Number.parseInt(one, 10));
  switch (chosen.how) {
    case "every":
      return { kind: "every", minutes: Math.round(chosen.every) * (chosen.unit === "days" ? 1440 : chosen.unit === "hours" ? 60 : 1) };
    case "daily":
      return { kind: "cron", line: `${minute ?? 0} ${hour ?? 9} * * *`, zone };
    case "weekly":
      return { kind: "cron", line: `${minute ?? 0} ${hour ?? 9} * * ${chosen.day}`, zone };
    case "once":
      return { kind: "once", at_ms: new Date(chosen.once).getTime() };
    default:
      return { kind: "cron", line: chosen.line.trim(), zone };
  }
}
