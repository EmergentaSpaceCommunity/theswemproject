import { useState } from "react";

import { materializeArtifact } from "./api";
import { keeping } from "./Navigator";
import { ProjectRail, SlotRail } from "./Rail";
import { RefChip } from "./RefChip";
import { Assets } from "./Assets";
import { Slots } from "./Slots.tsx";
import { Secrets, Tools } from "./Supplies";
import type { ArtifactView, ContentDescriptor, Envelope, LineView, NodeView, RevisionView, SelectionView, ToolView, UnitView, Viewing } from "./types";

interface Props {
  envelope: Envelope | null;
  envelopeError: string | null;
  viewing: Viewing;
  /// The tools the project's server declares, for the steps a rung offers.
  tools: ToolView[];
  /// Something was done to the project from here: re-read it now rather
  /// than when the watch notices.
  onChanged: () => void;
  onSelectRevision: (revisionRef: string) => void;
  onSelectBranch: (selectionRef: string, revisionRef: string) => void;
  /// Open the workbench of a slot, from its rail.
  onOpenSlot: (slot: string) => void;
}

/// The units of a slot's line, from the head revision the line names: what
/// that slot is made of right now. The slot is the line by the same name.
function unitsOfSlot(envelope: Envelope, slot: string): UnitView[] {
  const line = envelope.logical_ids.find((candidate) => candidate.logical_id === slot);
  if (!line) return [];
  const head = line.heads[0];
  const revision = line.revisions.find((candidate) => candidate.ref === head) ?? line.revisions[line.revisions.length - 1];
  return revision?.units ?? [];
}

function Sources({ envelope }: { envelope: Envelope }) {
  const artifacts = Object.entries(envelope.sources.artifacts);
  return (
    <section id="project-sources">
      <h3>Intent sources</h3>
      {artifacts.length === 0 && envelope.sources.groundings.length === 0 ? (
        <p className="k-muted">No source artifacts or groundings recorded.</p>
      ) : null}
      {artifacts.map(([reference, artifact]) => (
        <div key={reference} className="source-row k-row" data-ref={reference}>
          <RefChip value={reference} />
          <span>
            {artifact.media_type} · {artifact.byte_length} bytes · {artifact.type_ref}
          </span>
        </div>
      ))}
      {envelope.sources.groundings.map((grounding) => (
        <div key={grounding.ref} className="source-row k-row grounding" data-ref={grounding.ref}>
          <RefChip value={grounding.ref} />
          <span>
            {grounding.grounding.target_pointer} ← {grounding.grounding.interpretation}
          </span>
        </div>
      ))}
    </section>
  );
}

/// The recorded projections of the intent: the lens each was read through,
/// the entities it declares with their derived status, the reasoning. A
/// projection another one names as parent is superseded, not stale.
function Projections({ envelope }: { envelope: Envelope }) {
  const projections = envelope.sources.projections ?? [];
  if (projections.length === 0) return null;
  return (
    <section id="project-projections">
      <h3>Projections</h3>
      {projections.map((view) => {
        const superseded = (view.superseded_by ?? []).length > 0;
        const stale = view.declared.filter((entity) => entity.status !== "current").length;
        const status = superseded ? "superseded" : stale > 0 ? "stale" : "current";
        return (
          <article key={view.ref} className="projection-row" data-ref={view.ref} data-lens={view.projection.lens_ref} data-status={status}>
            <header>
              <RefChip value={view.ref} />
              <code>{view.projection.lens_ref}</code>
              <small>{status}</small>
            </header>
            <ul className="declared">
              {view.declared.map((entity) => (
                <li key={`${entity.kind}:${entity.name}`} className="declared-row" data-kind={entity.kind} data-name={entity.name} data-ref={entity.ref} data-status={entity.status}>
                  <code>
                    {entity.kind}:{entity.name}
                  </code>{" "}
                  {entity.status}
                </li>
              ))}
            </ul>
            <blockquote className="reasoning">{view.projection.reasoning}</blockquote>
          </article>
        );
      })}
    </section>
  );
}

/// The model plane of the viewed revision as the kernel reads it: every
/// unit with its intent, ports and effects, the connections, the roots and
/// the open ports. A unit no projection declares says so.
/// Why a unit exists: the claim a pin records, the fragment it quotes and the
/// source it quotes from. Every value here is the Cycle's own derivation
/// against this revision - which pin belongs to which unit is never decided
/// here. A pin whose value has moved keeps its claim and loses its quote.
function Provenance({ node, envelope }: { node: NodeView; envelope: Envelope }) {
  const pins = node.grounded_by ?? [];
  if (pins.length === 0) {
    return (
      <p className="k-muted provenance-debt">
        Nothing anchors this unit to the intent yet.
      </p>
    );
  }
  const record = (reference: string) =>
    envelope.sources.groundings.find((entry) => entry.ref === reference)?.grounding ?? null;
  return (
    <div className="provenance-block">
      {pins.map((pin) => {
        const claim = record(pin.ref);
        const source = claim ? envelope.sources.artifacts[claim.source_ref] : undefined;
        return (
          <div key={pin.ref} className="provenance" data-grounding-ref={pin.ref} data-state={pin.state}>
            <RefChip value={pin.ref} />
            <span className="provenance-state">{pin.state.replace("_", " ")}</span>
            {claim ? <p className="provenance-claim">{claim.interpretation}</p> : null}
            {pin.citation?.quote ? (
              <blockquote className="provenance-quote">{pin.citation.quote}</blockquote>
            ) : null}
            {claim ? (
              <small className="k-muted">
                {source
                  ? `from ${source.media_type} · ${source.byte_length} bytes`
                  : "the source is not listed among the project's inputs"}
              </small>
            ) : null}
          </div>
        );
      })}
      <small className="k-muted provenance-caveat">
        A preserved reference proves no coverage of the intent, no correctness of the
        interpretation and no sufficiency of an acceptance policy. Accepting the requirement
        itself remains a separate check.
      </small>
    </div>
  );
}

function Units({ revision, envelope }: { revision: RevisionView; envelope: Envelope }) {
  const model = revision.model ?? null;
  const unprojected = new Set(revision.unprojected ?? []);
  return (
    <section id="project-units" data-revision-ref={revision.ref} data-unit-count={model?.nodes.length ?? 0}>
      <h3>Units of the model</h3>
      {revision.model_error ? <p className="bad">{revision.model_error}</p> : null}
      {!model ? <p className="k-muted">This revision carries no model plane.</p> : null}
      {model ? (
        <div className="units">
          {model.nodes.map((node) => (
            <article key={node.id} className="unit-card" data-node-id={node.id} data-type-ref={node.type_ref} data-projected={unprojected.has(node.id) ? "false" : "true"}>
              <header>
                <strong>{node.id}</strong>
                <code>{node.type_ref}</code>
                {unprojected.has(node.id) ? <small className="bad">unprojected</small> : null}
              </header>
              {node.intent ? <p className="intent">{node.intent}</p> : <p className="k-muted">no intent recorded</p>}
              <Provenance node={node} envelope={envelope} />
              <div className="ports">
                {node.inputs.map((port) => (
                  <code key={`in:${port.name}`} className="port port-in" data-port={port.name} data-type-ref={port.type_ref}>
                    in {port.name}
                  </code>
                ))}
                {node.outputs.map((port) => (
                  <code key={`out:${port.name}`} className="port port-out" data-port={port.name} data-type-ref={port.type_ref}>
                    out {port.name}
                  </code>
                ))}
                {node.effects.map((effect) => (
                  <code key={`effect:${effect.interface}`} className="port port-effect" data-effect={effect.interface}>
                    effect {effect.interface} ({effect.capability_class})
                  </code>
                ))}
              </div>
            </article>
          ))}
          {model.connections.map((connection) => {
            const [from, to] = connection.split(" -> ");
            return (
              <div key={connection} className="connection-row" data-from={from} data-to={to}>
                {from} → {to}
              </div>
            );
          })}
          {Object.entries(model.roots).map(([root, port]) => (
            <div key={root} className="root-row" data-root={root} data-port={port}>
              <code>{root}</code> ← {port}
            </div>
          ))}
          {model.unbound_inputs.map((port) => (
            <div key={port} className="open-port-row" data-port={port}>
              open input {port}
            </div>
          ))}
        </div>
      ) : null}
    </section>
  );
}

function Revisions({ line, viewing, onSelectRevision }: { line: LineView; viewing: Viewing; onSelectRevision: (ref: string) => void }) {
  return (
    <section id="project-revision" data-logical-id={line.logical_id} data-revision-ref={viewing.revision_ref ?? ""}>
      <h3>Revisions of {line.logical_id}</h3>
      <div className="rows">
        {line.revisions.map((revision) => (
          <div
            key={revision.ref}
            className={`revision-row k-row k-selectable${revision.ref === viewing.revision_ref ? " active k-active" : ""}`}
            data-ref={revision.ref}
            data-head={line.heads.includes(revision.ref) ? "true" : "false"}
            onClick={() => onSelectRevision(revision.ref)}
          >
            <RefChip value={revision.ref} active={revision.ref === viewing.revision_ref} />
            <small>
              {line.heads.includes(revision.ref) ? "head" : "superseded"} · {revision.parents.length} parents ·{" "}
              {revision.selection_refs.length} deliveries
            </small>
          </div>
        ))}
      </div>
    </section>
  );
}

function BranchCard({ selection, isTip, active, onSelect }: { selection: SelectionView; isTip: boolean; active: boolean; onSelect: () => void }) {
  const decision = selection.obligations?.acceptance?.decision ?? null;
  const open = selection.obligations?.requirements.filter((requirement) => requirement.status !== "satisfied") ?? [];
  // Model-plane obligations: the policy rows are already the open requirements above.
  const modelRows = selection.obligations?.obligations.filter((obligation) => obligation.boundary === "model") ?? [];
  const ready = selection.obligations?.ready ?? false;
  return (
    <article
      className={`branch-card${active ? " active k-active" : ""}`}
      data-selection-ref={selection.ref}
      data-decision={decision ?? (selection.obligations_error ? "error" : "undecided")}
      data-ready={ready ? "true" : "false"}
      onClick={onSelect}
    >
      <header>
        <RefChip value={selection.ref} active={active} />
        <span className={`decision decision-${decision ?? "none"}`}>
          {decision ?? selection.obligations?.acceptance_blocked_by ?? selection.obligations_error ?? "no evidence"}
        </span>
        <small>{isTip ? "open branch" : `superseded by ${selection.superseded_by.length}`}</small>
      </header>
      <div className="roots">
        {selection.selection.roots.map((root) => (
          <code key={root} className="root">
            {root}
          </code>
        ))}
      </div>
      {open.length > 0 || modelRows.length > 0 ? (
        <ul className="obligations">
          {modelRows.map((obligation) => (
            <li key={`${obligation.required_predicate}:${obligation.introduced_by}`} className="obligation-row" data-status={obligation.blocking ? "blocking" : "open"}>
              <span>{obligation.boundary}</span> {obligation.introduced_by}: {obligation.required_predicate}
              {obligation.blocking ? " (blocking)" : ""}
            </li>
          ))}
          {open.map((requirement) => (
            <li key={requirement.constraint_ref} className="obligation-row" data-status={requirement.status}>
              <span>{requirement.subject_root}</span> {requirement.status}: {requirement.reason}
            </li>
          ))}
        </ul>
      ) : null}
    </article>
  );
}

/// One listed artifact: its record facts, and - once the operator asks the
/// host to bring its bytes in - the descriptor the bytes are served from, as
/// a download link and a browser-native preview by media type.
function ArtifactRow({ serverName, selectionRef, port, artifact }: { serverName: string | null; selectionRef: string; port: string; artifact: ArtifactView }) {
  const [descriptor, setDescriptor] = useState<ContentDescriptor | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const digest = artifact.content_digest;
  const save = async () => {
    if (!serverName) return;
    setBusy(true);
    setError(null);
    try {
      const result = await materializeArtifact<ContentDescriptor>(serverName, digest, selectionRef);
      setDescriptor(result.descriptor);
    } catch (failure: unknown) {
      setError(String(failure));
    } finally {
      setBusy(false);
    }
  };
  const served = descriptor ? `/api/content/${descriptor.descriptor_id}` : null;
  return (
    <div className="artifact-row k-row" data-ref={digest} data-descriptor-id={descriptor?.descriptor_id ?? ""}>
      <code>{port}</code> {artifact.media_type} · {artifact.byte_length} bytes
      {served ? (
        <>
          <a id={`artifact-download-${digest}`} className="artifact-download" href={`${served}?download=1`} download={descriptor?.name}>
            Download {descriptor?.name}
          </a>
          {artifact.media_type.startsWith("audio/") ? <audio id={`artifact-preview-${digest}`} controls src={served} /> : null}
        </>
      ) : (
        <button id={`artifact-save-${digest}`} className="artifact-save" type="button" disabled={busy || !serverName} onClick={save}>
          {busy ? "Fetching…" : "Save"}
        </button>
      )}
      {error ? <span className="bad">{error}</span> : null}
    </div>
  );
}

function Artifacts({ selection, serverName }: { selection: SelectionView; serverName: string | null }) {
  const executions = Object.entries(selection.executions);
  return (
    <section id="project-artifacts">
      <h3>Produced artifacts</h3>
      {executions.length === 0 ? <p className="k-muted">No closure produced artifacts for this branch yet.</p> : null}
      {executions.map(([reference, execution]) => (
        <div key={reference} className="execution-row k-row" data-ref={reference}>
          <RefChip value={reference} />
          <span>
            {execution.roots.join(", ")} · {execution.result?.kind ?? execution.error ?? "unknown"}
          </span>
          <div className="rows">
            {Object.entries(execution.produced_artifacts).map(([port, artifact]) => (
              <ArtifactRow key={port} serverName={serverName} selectionRef={selection.ref} port={port} artifact={artifact} />
            ))}
          </div>
        </div>
      ))}
    </section>
  );
}

function Evidence({ selection }: { selection: SelectionView }) {
  const requirements = selection.obligations?.requirements ?? [];
  return (
    <section id="project-evidence">
      <h3>Evidence</h3>
      {selection.obligations_error ? <p className="bad">{selection.obligations_error}</p> : null}
      {requirements.map((requirement) => (
        <div key={requirement.constraint_ref} className="evidence-row k-row" data-ref={requirement.constraint_ref} data-status={requirement.status}>
          <RefChip value={requirement.constraint_ref} />
          <span>
            {requirement.predicate_contract_ref} on {requirement.subject_root}: {requirement.status}
          </span>
          {(requirement.source_provenance ?? []).length > 0 ? (
            <div className="rows provenance-rows">
              <small className="k-muted">Required because of</small>
              {(requirement.source_provenance ?? []).map((entry) => (
                <span key={entry.ref} className="provenance" data-ref={entry.ref} data-kind={entry.kind}>
                  <RefChip value={entry.ref} />
                  <span className="provenance-state">{entry.kind}</span>
                </span>
              ))}
            </div>
          ) : null}
          <div className="rows">
            <small className="k-muted">
              {requirement.evidence_refs.length > 0
                ? "Observations recorded for this requirement. Whether one applies to the revision you are viewing is computed separately, from its view spec, environment and policy."
                : "No observation recorded for this requirement yet."}
            </small>
            {requirement.evidence_refs.map((reference) => (
              <RefChip key={reference} value={reference} />
            ))}
          </div>
        </div>
      ))}
    </section>
  );
}

/// Project Home: sources, the viewed revision line, delivery branches with
/// their derived decision and open obligations, then artifacts and evidence
/// of the viewed branch. Everything shown is a record ref the server declared.
export function ProjectHome({ envelope, envelopeError, viewing, tools, onChanged, onSelectRevision, onSelectBranch, onOpenSlot }: Props) {
  if (envelopeError) {
    return (
      <main id="project-home" className="main">
        <p className="bad">{envelopeError}</p>
      </main>
    );
  }
  if (!envelope) {
    return (
      <main id="project-home" className="main">
        <section id="project-empty">
          <h1>Open a project without starting an agent.</h1>
          {/* The first person here has nothing to choose from: telling them
              to choose a declared project server names an action the screen
              does not offer yet, and the one it does offer is on the left. */}
          <p>Pick a project on the left, or name a new one there. Its records are read as they are; nothing here is stored by the Workbench.</p>
        </section>
      </main>
    );
  }
  const line = envelope.logical_ids.find((candidate) => candidate.logical_id === viewing.logical_id) ?? null;
  const selection = line?.selections.find((candidate) => candidate.ref === viewing.selection_ref) ?? null;
  const revision = line?.revisions.find((candidate) => candidate.ref === viewing.revision_ref) ?? null;
  const acting = viewing.server_name ? { serverName: viewing.server_name, tools, onChanged } : null;
  return (
    <main id="project-home" className="main" data-record-set-digest={envelope.record_set_digest}>
      <header className="main-header">
        <strong>Project home</strong>
        <span className="k-muted">
          {keeping(envelope.persistence)} · <RefChip value={envelope.record_set_digest} />
        </span>
      </header>
      <div className="home-body">
        {/* The rail is the door: the project's own ladder, then one rail per
            slot with the door of its line and the workbench that projects it.
            Everything a person does here is a rung's action. */}
        <ProjectRail ladder={envelope.ladder} acting={acting} />
        {envelope.slots && viewing.server_name ? (
          <section id="slot-rails" aria-label="Slots">
            {envelope.slots.slots.map((slot) => (
              <SlotRail
                key={slot.slot}
                slot={slot}
                units={unitsOfSlot(envelope, slot.slot)}
                projectLadder={envelope.ladder}
                envelope={envelope}
                acting={acting}
                onOpen={onOpenSlot}
              />
            ))}
            <Slots serverName={viewing.server_name} slots={envelope.slots} onChanged={onChanged} />
          </section>
        ) : null}
        {viewing.server_name ? (
          <Assets
            serverName={viewing.server_name}
            assets={envelope.assets ?? { assets: [] }}
            onChanged={onChanged}
          />
        ) : null}
        {/* The records themselves: what the rails are derived from. A person
            works from the rails; these are for whoever needs the digests. */}
        <details id="developer-diagnostics">
          <summary>Records and diagnostics</summary>
          <div className="diagnostics-body">
            <Tools />
            {viewing.server_name ? <Secrets serverName={viewing.server_name} /> : null}
            <Sources envelope={envelope} />
            <Projections envelope={envelope} />
            {line ? <Revisions line={line} viewing={viewing} onSelectRevision={onSelectRevision} /> : null}
            {revision ? <Units revision={revision} envelope={envelope} /> : null}
            {line ? (
              <section id="project-branches">
                <h3>Delivery branches</h3>
                {line.selections.map((candidate) => (
                  <BranchCard
                    key={candidate.ref}
                    selection={candidate}
                    isTip={line.branch_tips.includes(candidate.ref)}
                    active={candidate.ref === viewing.selection_ref}
                    onSelect={() => onSelectBranch(candidate.ref, candidate.selection.model_revision_ref)}
                  />
                ))}
              </section>
            ) : null}
            {selection ? <Artifacts selection={selection} serverName={viewing.server_name} /> : null}
            {selection ? <Evidence selection={selection} /> : null}
          </div>
        </details>
      </div>
    </main>
  );
}
