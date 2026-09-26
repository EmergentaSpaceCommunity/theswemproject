import { useState } from "react";

import { Packages } from "./Packages";
import { useRecipes } from "./seeds";
import { RefChip } from "./RefChip";
import type { Envelope, ProjectSourceView, Viewing } from "./types";

interface Props {
  projects: ProjectSourceView[];
  projectsError: string | null;
  envelope: Envelope | null;
  viewing: Viewing;
  onSelectProject: (serverName: string) => void;
  onSelectLine: (logicalId: string) => void;
  onSelectBranch: (selectionRef: string, revisionRef: string) => void;
  onRefresh: () => void;
  /// A seed, when the person picked one: what the project starts as, run
  /// after what was asked is recorded.
  onCreateProject: (name: string, intent: string, seed: string | null) => void;
  creating: boolean;
  createError: string | null;
}

/// Three flat lists - projects, revision lines, delivery branches. No folder
/// tree, no progress: readiness lives on the branch card, per branch.
/// Whether this project's records are kept, in the words that matter to the
/// person choosing it. The server's own word (`journal`, `process_memory`)
/// tells them nothing, and the one case they must not miss - a project that
/// is not being kept at all - looked like the other one. An unknown word is
/// shown as it came rather than guessed at.
export function keeping(persistence: string | undefined): string {
  // Not asked yet: the row says its name and nothing the host would have
  // to invent. The caption arrives once the project has been opened.
  if (persistence === undefined) return "";
  if (persistence === "journal") return "kept on disk";
  if (persistence === "process_memory") return "not kept: it goes when the product stops";
  if (persistence === "unsupported_transport") return "this product cannot read it";
  return persistence;
}

export function Navigator(props: Props) {
  const { projects, projectsError, envelope, viewing } = props;
  const line = envelope?.logical_ids.find((candidate) => candidate.logical_id === viewing.logical_id) ?? null;
  // What an installed package says a project of its kind starts as. Picking
  // one offers its own words for what was asked; they stay the person's to
  // replace, because the seed's steps run after the intent is recorded and
  // never over it.
  const [allRecipes, reloadRecipes] = useRecipes();
  const seeds = allRecipes.filter((recipe) => recipe.starts_a_project);
  const [seed, setSeed] = useState("");
  const [intent, setIntent] = useState("");
  return (
    <aside id="project-navigator" className="rail" aria-label="Project navigator">
      <div className="brand">
        <span className="brand-mark" />
        SWEM
      </div>
      <div className="k-eyebrow">Projects</div>
      {projectsError ? <div className="bad">{projectsError}</div> : null}
      {projects.length === 0 ? (
        <p className="k-muted">No project yet. Name one below; it opens without an agent.</p>
      ) : null}
      {/* A project is made here, from its name and what was asked: the host
          creates its workspace and journal and serves them itself, and the
          intent is the first record in it - nothing opens before it. */}
      <form
        className="new-project"
        onSubmit={(event) => {
          event.preventDefault();
          const data = new FormData(event.currentTarget);
          const name = data.get("name");
          if (typeof name === "string" && name.trim() && intent.trim()) {
            props.onCreateProject(name.trim(), intent.trim(), seed || null);
            event.currentTarget.reset();
            setIntent("");
            setSeed("");
          }
        }}
      >
        <input id="new-project-name" name="name" placeholder="New project name" aria-label="New project name" required />
        {seeds.length > 0 ? (
          <select
            id="new-project-seed"
            aria-label="Start from"
            value={seed}
            onChange={(event) => {
              const picked = event.target.value;
              setSeed(picked);
              const chosen = seeds.find((candidate) => candidate.name === picked);
              // Only ever fills an empty box: what the person wrote is theirs.
              if (chosen?.intent_example && !intent.trim()) setIntent(chosen.intent_example);
            }}
          >
            <option value="">Start from nothing</option>
            {seeds.map((candidate) => (
              <option key={candidate.name} value={candidate.name}>
                {candidate.title} · {candidate.module}
              </option>
            ))}
          </select>
        ) : null}
        {seed ? (
          <p id="new-project-seed-summary" className="k-muted">
            {seeds.find((candidate) => candidate.name === seed)?.summary}
          </p>
        ) : null}
        <textarea
          id="new-project-intent"
          name="intent"
          rows={3}
          placeholder="What do you want to make? One or two sentences: the piece, the page, the thing."
          aria-label="What was asked"
          value={intent}
          onChange={(event) => setIntent(event.target.value)}
          required
        />
        <button id="create-project" type="submit" disabled={props.creating}>
          {props.creating ? "Creating…" : "Create project"}
        </button>
      </form>
      {props.createError ? <div className="bad">{props.createError}</div> : null}
      <div className="rows">
        {projects.map((project) => (
          <button
            key={project.server_name}
            className={`project-row k-pick${project.server_name === viewing.server_name ? " active k-active" : ""}`}
            data-server-name={project.server_name}
            data-available={project.available === null ? undefined : project.available ? "true" : "false"}
            onClick={() => props.onSelectProject(project.server_name)}
          >
            <strong>{project.server_name}</strong>
            <small title={project.connection_scope}>{keeping(project.persistence)}</small>
          </button>
        ))}
      </div>
      {envelope ? (
        <>
          <div className="k-eyebrow">Revision lines</div>
          {envelope.logical_ids.length === 0 ? (
            <p id="lines-empty" className="k-caption k-muted">
              None yet. Beginning the work in a slot opens one.
            </p>
          ) : null}
          <div className="rows">
            {envelope.logical_ids.map((candidate) => (
              <button
                key={candidate.logical_id}
                className={`line-row k-pick${candidate.logical_id === viewing.logical_id ? " active k-active" : ""}`}
                data-logical-id={candidate.logical_id}
                onClick={() => props.onSelectLine(candidate.logical_id)}
              >
                <strong>{candidate.logical_id}</strong>
                <small>
                  {candidate.revisions.length} revisions · {candidate.selections.length} deliveries
                </small>
              </button>
            ))}
          </div>
        </>
      ) : null}
      {line ? (
        <>
          <div className="k-eyebrow">Delivery branches</div>
          <div className="rows">
            {line.selections.map((selection) => (
              <button
                key={selection.ref}
                className={`branch-row k-pick${selection.ref === viewing.selection_ref ? " active k-active" : ""}`}
                data-selection-ref={selection.ref}
                data-tip={line.branch_tips.includes(selection.ref) ? "true" : "false"}
                onClick={() => props.onSelectBranch(selection.ref, selection.selection.model_revision_ref)}
              >
                <RefChip value={selection.ref} />
                <small>
                  {selection.obligations?.acceptance?.decision ?? (selection.obligations_error ? "error" : "open")}
                  {line.branch_tips.includes(selection.ref) ? " · tip" : " · superseded"}
                </small>
              </button>
            ))}
          </div>
        </>
      ) : null}
      {/* What this SWEM can do at all, and how to give it more: a package is
          how a domain arrives, so it belongs beside the projects rather than
          in a project. */}
      <Packages onInstalled={reloadRecipes} />
      <button id="project-refresh" onClick={props.onRefresh} style={{ marginTop: 16 }}>
        Refresh
      </button>
    </aside>
  );
}
