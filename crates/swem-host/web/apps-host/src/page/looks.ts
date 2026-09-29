// What an agent has where it lives, as the host found it.

import { fetchJson } from "../http.ts";

export interface ProgramFound {
  name: string;
  path?: string | null;
  version?: string | null;
}

export interface LookInside {
  profile_id: string;
  looked_ms: number;
  engine: {
    starts: boolean;
    name?: string | null;
    version?: string | null;
    started_in_ms?: number | null;
    signed_in?: boolean | null;
    answered_in_ms?: number | null;
    said?: string;
  };
  machine?: {
    system: string;
    architecture: string;
    user?: string | null;
    processors: number;
    workspace_writable?: boolean | null;
    workspace_owned?: boolean | null;
    workspace_free_bytes?: number | null;
    programs: ProgramFound[];
  } | null;
  containers: boolean;
  containers_said?: string;
}

const part = encodeURIComponent;

export const looks = {
  async kept(profile: string): Promise<LookInside | null> {
    return (await fetchJson<{ look: LookInside | null }>(`/api/profiles/${part(profile)}/look`)).look;
  },
  async now(profile: string): Promise<LookInside> {
    return (await fetchJson<{ look: LookInside }>(`/api/profiles/${part(profile)}/look`, { method: "POST" })).look;
  },
};

/// How long something took, as a person says it.
export const took = (ms: number): string => (ms < 950 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`);

/// The number of a version out of what a program says of itself:
/// "git version 2.47.0" is 2.47.0.
export function versionIn(said: string | null | undefined): string {
  return /\d+(\.\d+)+/.exec(said ?? "")?.[0] ?? "";
}

/// The programs that are there, by name and version, as one line.
export function programsFound(programs: ProgramFound[], names: string[]): { names: string; versions: string } {
  const there = names.flatMap((name) => programs.filter((one) => one.name === name && one.path));
  return {
    names: there.map((one) => (one.name === "python3" ? "python" : one.name)).join(", "),
    versions: there.map((one) => versionIn(one.version) || "there").join(" · "),
  };
}
