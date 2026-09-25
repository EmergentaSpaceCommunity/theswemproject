import { useEffect, useState } from "react";

export interface EnvelopeRead<T> {
  uri: string;
  text: string;
  json: T;
}

async function failure(response: Response): Promise<Error> {
  const body = await response.text();
  try {
    const parsed = JSON.parse(body) as { error?: string };
    return new Error(parsed.error ?? body);
  } catch {
    return new Error(body || `${response.status}`);
  }
}

export async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, init);
  if (!response.ok) throw await failure(response);
  return (await response.json()) as T;
}

/// A verbatim resource read: the host forwards the server's text and names
/// the exact resource URI in a header.
export async function readEnvelope<T>(path: string): Promise<EnvelopeRead<T>> {
  const response = await fetch(path);
  if (!response.ok) throw await failure(response);
  const text = await response.text();
  return { uri: response.headers.get("x-swem-resource-uri") ?? "", text, json: JSON.parse(text) as T };
}

/// Ask the host to bring one listed artifact's bytes into its content store
/// (read from the server at the URI the envelope declares, digest-verified);
/// the descriptor names where the bytes are served from now on.
///
/// The selection being read travels with the request. The same bytes can be
/// produced by several branches, so without it the host serves them but names
/// no producing execution rather than naming a foreign one.
export async function materializeArtifact<T>(
  serverName: string,
  digest: string,
  /// The branch the bytes are read under; absent for a project-level asset,
  /// whose ref means the same thing in every selection.
  selectionRef?: string,
): Promise<{ descriptor: T; uri: string }> {
  const response = await fetch(`/api/projects/${encodeURIComponent(serverName)}/artifacts/${encodeURIComponent(digest)}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(selectionRef === undefined ? {} : { selection_ref: selectionRef }),
  });
  if (!response.ok) throw await failure(response);
  return { descriptor: (await response.json()) as T, uri: response.headers.get("x-swem-resource-uri") ?? "" };
}

export interface Resource<T> {
  data: T | null;
  uri: string | null;
  error: string | null;
  loading: boolean;
}

/// Fetch-on-demand cache for one resource path; `version` forces a re-read.
///
/// What it holds is always this `path`'s: a re-read of the same path keeps
/// what is on screen while the new bytes arrive, and a change of path drops
/// it. Keeping it across a change would show one project's lines under
/// another project's selection until the fetch landed - a reading a person
/// could act on, and a second source of truth for as long as it lasted.
export function useEnvelope<T>(path: string | null, version: number): Resource<T> {
  const [state, setState] = useState<Resource<T> & { path: string | null }>({
    data: null,
    uri: null,
    error: null,
    loading: false,
    path: null,
  });
  useEffect(() => {
    if (!path) {
      setState({ data: null, uri: null, error: null, loading: false, path: null });
      return;
    }
    let cancelled = false;
    setState((previous) =>
      previous.path === path
        ? { ...previous, loading: true, error: null }
        : { data: null, uri: null, error: null, loading: true, path },
    );
    readEnvelope<T>(path)
      .then((read) => {
        if (!cancelled) setState({ data: read.json, uri: read.uri, error: null, loading: false, path });
      })
      .catch((error: unknown) => {
        if (!cancelled) setState({ data: null, uri: null, error: String(error), loading: false, path });
      });
    return () => {
      cancelled = true;
    };
  }, [path, version]);
  return { data: state.data, uri: state.uri, error: state.error, loading: state.loading };
}

/// One step a person takes on the project without an agent: a tool the
/// Cycle itself offered, called with the arguments the form produced. The
/// host gates it exactly as it gates an App's calls.
/// Run one recipe a package brought on one project. Every step is an ordinary
/// tool call behind the same gate; a step that fails comes back inside the
/// answer rather than as a failure of the run, so the person sees which step.
export async function runRecipe(serverName: string, recipe: string): Promise<RecipeRun> {
  return fetchJson<RecipeRun>(
    `/api/projects/${encodeURIComponent(serverName)}/recipes/${encodeURIComponent(recipe)}/run`,
    { method: "POST", headers: { "content-type": "application/json" }, body: "{}" },
  );
}

export interface RecipeStepRun {
  note: string;
  tool: string;
  /// `done`, `failed`, or `not run` for a step after the one that failed.
  outcome: string;
  error?: string;
}

export interface RecipeRun {
  recipe: string;
  title: string;
  completed: boolean;
  steps: RecipeStepRun[];
}

/// The sentence a person is owed when a run stopped: which step, and why.
export function recipeFailure(run: RecipeRun): string | null {
  if (run.completed) return null;
  const stopped = run.steps.find((step) => step.outcome === "failed");
  return stopped ? `${run.title} stopped at "${stopped.note}": ${stopped.error ?? "the step failed"}` : null;
}

export async function callProjectTool(serverName: string, tool: string, args: Record<string, unknown>): Promise<unknown> {
  const response = await fetch(`/api/projects/${encodeURIComponent(serverName)}/tools/${encodeURIComponent(tool)}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ arguments: args }),
  });
  if (!response.ok) throw await failure(response);
  return (await response.json()) as unknown;
}
