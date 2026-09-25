import { useCallback, useEffect, useState } from "react";

import { Activity } from "./Activity";
import { DomainApp, useDomainApps } from "./DomainApps";
import { Collaborator } from "./Collaborator";
import { Navigator } from "./Navigator";
import { ProjectHome } from "./ProjectHome";
import { RailStrip } from "./Rail";
import { callProjectTool, fetchJson, recipeFailure, runRecipe, useEnvelope } from "./api";
import { slotTabs } from "./slots";
import type { Envelope, ProjectSourceView, Viewing } from "./types";
import { useConnectionId } from "./useConnection";
import { sessionStore, useSession } from "../agent/store.ts";

const NOTHING: Viewing = { server_name: null, logical_id: null, revision_ref: null, selection_ref: null };

/// The second entry point: a project opened from its records, without an
/// agent. Every selection here changes only `viewing`; the agent context
/// changes only through the explicit bind and clear actions.
export function ProjectSpace({ hidden }: { hidden: boolean }) {
  const [projects, setProjects] = useState<ProjectSourceView[]>([]);
  const [projectsError, setProjectsError] = useState<string | null>(null);
  const [viewing, setViewing] = useState<Viewing>(NOTHING);
  const [version, setVersion] = useState(0);
  // The binding is the session's, held by the Agent space's store: both
  // spaces show the same one, and only bind and clear write it.
  const { context, contextError } = useSession();
  // Which slot's workbench is open, by slot name: two slots of the same
  // module are two places, and the tab strip must not conflate them.
  const [openSlot, setOpenSlot] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const connectionId = useConnectionId();

  const loadProjects = useCallback(() => {
    fetchJson<ProjectSourceView[]>("/api/projects")
      .then((list) => {
        setProjects(list);
        setProjectsError(null);
      })
      .catch((error: unknown) => setProjectsError(String(error)));
  }, []);
  const envelopePath = viewing.server_name ? `/api/projects/${encodeURIComponent(viewing.server_name)}/envelope` : null;
  const envelope = useEnvelope<Envelope>(envelopePath, version);
  // Listed again once a project's envelope has arrived, and whenever its
  // records move: the row of a project nobody had opened says only its
  // name, and what the server says of itself is on the row only after the
  // server has been asked. Asking is the envelope read, so the list follows
  // the envelope, not the click - on the click the server has not answered
  // yet and the row would come back exactly as unknown as before.
  useEffect(loadProjects, [loadProjects, envelope.data, version]);
  const operations = envelope.data?.operations ?? [];
  const digest = envelope.data?.record_set_digest ?? null;

  // The project is watched, not refreshed. The host holds a request open until
  // this project's record set is no longer the one we hold, and every change
  // arrives the same way whoever made it: this person, their agent in another
  // process, or a stock MCP client over the same journal. Re-reading the
  // envelope is the whole response - the view is a projection, so there is no
  // delta to apply and nothing to reconcile.
  const [changed, setChanged] = useState<string | null>(null);
  useEffect(() => {
    const server = viewing.server_name;
    if (!server || hidden || !digest) return undefined;
    let watching = true;
    const controller = new AbortController();
    void (async () => {
      let held = digest;
      while (watching) {
        let next: {record_set_digest: string} | null;
        try {
          next = await fetchJson<{record_set_digest: string} | null>(
            `/api/projects/${encodeURIComponent(server)}/changes/next` +
              `?after=${encodeURIComponent(held)}&wait_ms=20000`,
            {signal: controller.signal},
          );
        } catch {
          return; // The watch ends with the project, the tab or the process.
        }
        if (!watching) return;
        if (!next) continue; // Nothing moved in that window; ask again.
        held = next.record_set_digest;
        setChanged(held);
        setVersion((current) => current + 1);
      }
    })();
    return () => { watching = false; controller.abort(); };
  }, [viewing.server_name, hidden, digest]);

  const loadContext = useCallback(() => {
    void sessionStore.loadContext();
  }, []);
  useEffect(loadContext, [loadContext, connectionId]);

  // Creating a project is the way in, and what was asked is the first
  // thing in it: the host makes the workspace and journal and serves them,
  // then records the intent through the action the project's own ladder
  // offers on its first rung - the surface names no tool and no type of
  // its own. Without the intent the Cycle opens nothing else.
  const createProject = (name: string, intent: string, seed: string | null) => {
    setCreating(true);
    setCreateError(null);
    fetch("/api/projects", {
      method: "POST",
      headers: {"content-type": "application/json"},
      body: JSON.stringify({name}),
    })
      .then(async (response) => {
        const body = await response.json();
        if (!response.ok) throw new Error(body?.error ?? response.statusText);
        return body as ProjectSourceView;
      })
      .then(async (project) => {
        const fresh = await fetchJson<Envelope>(`/api/projects/${encodeURIComponent(project.server_name)}/envelope`);
        const first = fresh.ladder?.rungs.find((rung) => rung.step === "vision")?.actions?.[0];
        if (!first) throw new Error("the project's ladder offers no way to record what was asked");
        await callProjectTool(project.server_name, first.tool, { ...(first.preset ?? {}), text: intent });
        // A seed runs after what was asked, never over it: its steps are for
        // a project that has its intent and nothing else.
        // The project exists either way; a seed that stopped says where.
        if (seed) {
          const stopped = recipeFailure(await runRecipe(project.server_name, seed));
          if (stopped) setCreateError(stopped);
        }
        return project;
      })
      .then((project) => {
        loadProjects();
        setOpenSlot(null);
        setViewing({server_name: project.server_name, logical_id: null, revision_ref: null, selection_ref: null});
      })
      .catch((error: unknown) => setCreateError(String(error)))
      .finally(() => setCreating(false));
  };

  const selectProject = (serverName: string) => {
    setOpenSlot(null);
    setViewing({ server_name: serverName, logical_id: null, revision_ref: null, selection_ref: null });
  };
  const selectLine = (logicalId: string) => {
  const line = envelope.data?.logical_ids.find((candidate) => candidate.logical_id === logicalId) ?? null;
    setViewing((previous) => ({
      ...previous,
      logical_id: logicalId,
      revision_ref: line?.heads[0] ?? null,
      selection_ref: null,
    }));
  };
  const selectRevision = (revisionRef: string) => {
    setViewing((previous) => ({ ...previous, revision_ref: revisionRef, selection_ref: null }));
  };
  const selectBranch = (selectionRef: string, revisionRef: string) => {
    setViewing((previous) => ({ ...previous, revision_ref: revisionRef, selection_ref: selectionRef }));
  };

  const project = projects.find((candidate) => candidate.server_name === viewing.server_name) ?? null;
  const bindable = Boolean(connectionId && project?.available === true && viewing.revision_ref);
  const bind = () => {
    if (!connectionId || !viewing.server_name || !viewing.revision_ref) return;
    void sessionStore.bindContext({
      server_name: viewing.server_name,
      revision_ref: viewing.revision_ref,
      selection_ref: viewing.selection_ref,
    });
  };
  const clear = () => {
    if (!connectionId) return;
    void sessionStore.clearContext();
  };

  const published = useDomainApps(viewing.server_name);
  // One workbench per slot: what the project is made of, not what the
  // binary happens to compile. It opens from the slot's rail, never from a
  // strip of its own.
  const tabs = slotTabs(envelope.data, published.apps);
  // A slot that was removed (here or by an agent) takes its surface with it.
  const open = tabs.find((tab) => tab.slot === openSlot) ?? null;
  const openView = envelope.data?.slots?.slots.find((slot) => slot.slot === openSlot) ?? null;
  const line = envelope.data?.logical_ids.find((candidate) => candidate.logical_id === viewing.logical_id) ?? null;
  return (
    <section id="project-space" className="space project-space" data-workbench-space="project" data-open-slot={open && openView ? open.slot : undefined} hidden={hidden}>
      <Navigator
        projects={projects}
        projectsError={projectsError}
        envelope={envelope.data}
        viewing={viewing}
        onSelectProject={selectProject}
        onSelectLine={selectLine}
        onSelectBranch={selectBranch}
        onCreateProject={createProject}
        creating={creating}
        createError={createError}
        onRefresh={() => {
          loadProjects();
          setVersion((current) => current + 1);
          loadContext();
        }}
      />
      {/* The main area is a workbench, chosen by a tab: the project itself, or
          one of the domain surfaces its server publishes. */}
      <main className="surface" aria-label="Project surface">
        {open && openView && viewing.server_name && open.uri ? (
          <>
            <RailStrip slot={openView} onBack={() => setOpenSlot(null)} />
            {/* Keyed by slot: switching to another slot opens that slot's
                workbench rather than leaving the previous one mounted. */}
            <DomainApp key={open.slot} server={viewing.server_name} uri={open.uri} slot={open.slot} changed={changed} />
          </>
        ) : (
          <ProjectHome
            envelope={envelope.data}
            envelopeError={envelope.error}
            viewing={viewing}
            tools={published.tools}
            onChanged={() => setVersion((current) => current + 1)}
            onSelectRevision={selectRevision}
            onSelectBranch={selectBranch}
            onOpenSlot={(slot) => {
              if (tabs.some((tab) => tab.slot === slot && tab.uri)) setOpenSlot(slot);
            }}
          />
        )}
      </main>
      <Collaborator
        viewing={viewing}
        connectionId={connectionId}
        context={context}
        contextError={contextError}
        bindable={bindable}
        onBind={bind}
        onClear={clear}
      />
      <Activity line={line} operations={operations} />
    </section>
  );
}
