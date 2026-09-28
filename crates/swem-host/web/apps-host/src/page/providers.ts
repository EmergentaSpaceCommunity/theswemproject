// What agents are put together from, as the host says it: the providers of
// models with whether each has its key, this machine as it was found, and
// the places on it an agent may live.

import { createStore } from "zustand/vanilla";

import type { DeclareModelProvider, ModelProviderView, SecretType } from "../agent/session.ts";
import { fetchJson } from "../http.ts";

export interface KeyHeld {
  variable: string;
  given_ms: number;
}

export interface ModelProviderStanding extends ModelProviderView {
  key?: KeyHeld | null;
  /// The profiles of the agents that answer from it.
  used_by: string[];
}

export interface EngineLook {
  standing: "not_found" | "not_ready" | "ready";
  version?: string | null;
  said: string;
}

export interface MachineLook {
  looked_ms: number;
  system: string;
  architecture: string;
  processors: number;
  memory_bytes: number;
  memory_free_bytes: number;
  disk_free_bytes?: number | null;
  podman: EngineLook;
  docker: EngineLook;
}

export interface HostStanding {
  id: string;
  name: string;
  kind: string;
  machine: string;
  an_agent_gets: string;
  used_by: string[];
  ready: boolean;
  said: string;
}

export interface SetUpStep {
  what: string;
  /// The command that does it, as it is run.
  command: string;
  state: "waiting" | "running" | "done" | "failed";
  said: string;
}

/// Setting containers up on this machine: what would be done, what is being
/// done, or how it ended.
export interface SetUp {
  plan_id: string;
  steps: SetUpStep[];
  effects: string[];
  blockers: string[];
  state: "offered" | "running" | "done" | "failed";
  said: string;
  hint?: string | null;
}

/// The machine containers run in, as a person sizes it.
export interface MachineWanted {
  cpus: number;
  memory_mib: number;
  disk_gib: number;
}

interface Providers {
  models: ModelProviderStanding[] | null;
  /// What keeps the keys, in the host's words.
  keptBy: string;
  /// What a provider can be opened by.
  kinds: SecretType[];
  machine: MachineLook | null;
  hosts: HostStanding[] | null;
  looking: boolean;
  settingUp: SetUp | null;
  problem: string;
}

export const providers = createStore<Providers>(() => ({
  models: null,
  keptBy: "",
  kinds: [],
  machine: null,
  hosts: null,
  looking: false,
  settingUp: null,
  problem: "",
}));

const part = encodeURIComponent;
const send = (method: string, body?: unknown): RequestInit => ({
  method,
  ...(body === undefined ? {} : { headers: { "content-type": "application/json" }, body: JSON.stringify(body) }),
});
const said = (error: unknown): string => (error instanceof Error ? error.message : String(error));

interface Hosts {
  machine: MachineLook | null;
  hosts: HostStanding[];
  setting_up?: SetUp | null;
}

let watching: ReturnType<typeof setTimeout> | null = null;
/// While containers are being set up, ask how it goes.
function watch(): void {
  if (watching !== null) return;
  const again = async () => {
    watching = null;
    await providing.hosts();
    if (providers.getState().settingUp?.state === "running") watching = setTimeout(again, 1500);
  };
  watching = setTimeout(again, 1000);
}

export const providing = {
  async models(): Promise<void> {
    try {
      const answer = await fetchJson<{ providers: ModelProviderStanding[]; kept_by: string | null; key_kinds?: SecretType[] }>("/api/model-providers");
      providers.setState({ models: answer.providers, keptBy: answer.kept_by ?? "", kinds: answer.key_kinds ?? [], problem: "" });
    } catch (error) {
      providers.setState({ problem: said(error) });
    }
  },
  /// Give a provider its key. What was typed is sent once and kept nowhere
  /// on the page.
  async giveKey(provider: string, value: string, variable?: string): Promise<void> {
    await fetchJson(`/api/model-providers/${part(provider)}/key`, send("PUT", { value, ...(variable ? { variable } : {}) }));
    await providing.models();
  },
  /// Add a provider, or correct one.
  async add(provider: DeclareModelProvider): Promise<void> {
    await fetchJson("/api/model-providers", send("POST", provider));
    await providing.models();
  },
  async forget(provider: string): Promise<void> {
    await fetchJson(`/api/model-providers/${part(provider)}`, send("DELETE"));
    await providing.models();
  },
  async takeKey(provider: string): Promise<void> {
    await fetchJson(`/api/model-providers/${part(provider)}/key`, send("DELETE"));
    await providing.models();
  },
  async hosts(): Promise<void> {
    try {
      const answer = await fetchJson<Hosts>("/api/hosts");
      providers.setState({ machine: answer.machine, hosts: answer.hosts, settingUp: answer.setting_up ?? null, problem: "" });
      if (answer.setting_up?.state === "running") watch();
    } catch (error) {
      providers.setState({ problem: said(error) });
    }
  },
  /// What setting containers up here would do, for a machine of that size.
  async planContainers(wanted: MachineWanted): Promise<SetUp> {
    const plan = await fetchJson<SetUp>("/api/hosts/this-machine/containers/plan", send("POST", wanted));
    providers.setState({ settingUp: plan });
    return plan;
  },
  /// Do what was offered, as a person agreed to it.
  async setContainersUp(plan: string): Promise<void> {
    const run = await fetchJson<SetUp>("/api/hosts/this-machine/containers/set-up", send("POST", { plan_id: plan }));
    providers.setState({ settingUp: run });
    watch();
  },
  async lookAgain(): Promise<void> {
    providers.setState({ looking: true, problem: "" });
    try {
      const answer = await fetchJson<Hosts>("/api/hosts/this-machine/look", send("POST"));
      providers.setState({ machine: answer.machine, hosts: answer.hosts });
    } catch (error) {
      providers.setState({ problem: said(error) });
    } finally {
      providers.setState({ looking: false });
    }
  },
};

const GIGABYTE = 2 ** 30;
/// A size as a person reads it on a machine: whole gigabytes.
export const gigabytes = (bytes: number): string => `${Math.round(bytes / GIGABYTE)} GB`;

/// When something was done, as a person says it on the day: the time, and
/// the date as well once it is another day.
export function when(ms: number, now: number = Date.now()): string {
  const then = new Date(ms);
  const time = then.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const sameDay = new Date(now).toDateString() === then.toDateString();
  return sameDay ? time : `${then.toLocaleDateString([], { day: "numeric", month: "short" })}, ${time}`;
}
