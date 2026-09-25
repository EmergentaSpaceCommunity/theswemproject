// The projection rung's own editor: what was asked on one side, the reading
// of it on the other, and the plan it declares as rows a person fills in.
//
// This is not a generic form over `project_vision`'s schema - that schema is
// arrays of objects and has no flat form. It is the one
// operation both a person and an agent perform, presented for a person: the
// same tool, the same arguments, the same record. Every domain word on the
// screen comes from the slot's own `projection` declaration, which the Cycle
// carries from the module's manifest; this file names none.

import { useEffect, useMemo, useState } from "react";

import { callProjectTool, materializeArtifact } from "./api";
import type { ContentDescriptor, Envelope, SketchKindView, SlotView } from "./types";

/// Every `[[kind:name]]` of a text, in order, without duplicates - the same
/// reading the Cycle performs on the reasoning it is given, so what a person
/// sees as rows is exactly what the record will have to declare.
export function tagsOf(reasoning: string): { kind: string; name: string }[] {
  const tags: { kind: string; name: string }[] = [];
  const pattern = /\[\[([^[\]:]+):([^[\]:]+)\]\]/g;
  for (const match of reasoning.matchAll(pattern)) {
    const kind = match[1];
    const name = match[2];
    if (!kind || !name) continue;
    if (tags.some((tag) => tag.kind === kind && tag.name === name)) continue;
    tags.push({ kind, name });
  }
  return tags;
}

/// What a person filled in for one tag, before it becomes a sketch.
interface Row {
  within: string;
  intent: string;
  fields: Record<string, string>;
}

function blankRow(declaration: SketchKindView | undefined): Row {
  const fields: Record<string, string> = {};
  for (const field of declaration?.fields ?? []) {
    fields[field.name] = field.default === undefined || field.default === null ? "" : String(field.default);
  }
  return { within: "", intent: "", fields };
}

/// A field's value as the tool takes it: a count stays a count, everything
/// else travels as the text a person typed. The module declared what the
/// word is worth by default, and its type is the type of that default.
function fieldValue(raw: string, fallback: unknown): unknown {
  if (raw.trim() === "") return fallback ?? raw;
  if (typeof fallback === "number") {
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? parsed : raw;
  }
  if (typeof fallback === "boolean") return raw === "true";
  return raw;
}

/// What this project recorded about what was asked: the refs the reading
/// will cite, and - once the host has brought the bytes into its content
/// store - the words themselves, so a person reads what was asked while
/// writing the reading of it. The refs stand on the envelope alone: bytes
/// that will not come back must not stop a person from writing.
function useIntentTexts(
  serverName: string,
  envelope: Envelope,
  typeRef: string | null,
): { ref: string; text: string | null }[] {
  const [texts, setTexts] = useState<Record<string, string>>({});
  const refs = useMemo(
    () =>
      Object.entries(envelope.sources.artifacts)
        .filter(([, artifact]) => typeRef !== null && artifact.type_ref === typeRef)
        .map(([reference, artifact]) => ({ reference, digest: artifact.content_digest })),
    [envelope, typeRef],
  );
  const key = refs.map((entry) => entry.reference).join(",");
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const read: Record<string, string> = {};
      for (const entry of refs) {
        try {
          const result = await materializeArtifact<ContentDescriptor>(serverName, entry.digest);
          // The host serves what it materialized under its content store; the
          // artifact's own `swem://` uri names the bytes, it does not fetch them.
          const response = await fetch(`/api/content/${encodeURIComponent(result.descriptor.descriptor_id)}`);
          if (response.ok) read[entry.reference] = await response.text();
        } catch {
          // The bytes are a courtesy here: the reading is written either way.
        }
      }
      if (!cancelled) setTexts(read);
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serverName, key]);
  return refs.map((entry) => ({ ref: entry.reference, text: texts[entry.reference] ?? null }));
}

interface Props {
  serverName: string;
  slot: SlotView;
  envelope: Envelope;
  /// The type an intent text is recorded under, as the vision rung's own
  /// action declares it; the surface never decides what an intent is.
  intentTypeRef: string | null;
  onChanged: () => void;
}

/// The editor. It appears on the projection rung of a slot whose module
/// declares a lens, and it is the person's half of the one step an agent
/// takes with the same tool.
export function Decompose({ serverName, slot, envelope, intentTypeRef, onChanged }: Props) {
  const projection = slot.projection;
  const [open, setOpen] = useState(false);
  const [reasoning, setReasoning] = useState("");
  const [rows, setRows] = useState<Record<string, Row>>({});
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const texts = useIntentTexts(serverName, envelope, open ? intentTypeRef : null);
  const tags = useMemo(() => tagsOf(reasoning), [reasoning]);
  const sketches = useMemo(() => projection?.sketches ?? [], [projection]);
  const declarationOf = (kind: string): SketchKindView | undefined => sketches.find((sketch) => sketch.kind === kind);

  /// The readings of THIS slot's work, live. Which readings belong to a slot
  /// is the Cycle's derivation - its projection rung names them - and one a
  /// re-projection replaced is history, not something to cite or supersede.
  const live = useMemo(() => {
    const rung = slot.ladder?.rungs.find((step) => step.step === "projection");
    const ours = new Set(rung?.refs ?? []);
    return (envelope.sources.projections ?? []).filter(
      (view) => ours.has(view.ref) && (view.superseded_by ?? []).length === 0,
    );
  }, [envelope, slot]);

  /// What an earlier projection already declared and still holds: a tag
  /// naming one of these cites it instead of asking for a second copy.
  const cited = useMemo(() => {
    const table: Record<string, string> = {};
    for (const view of live) {
      for (const entity of view.declared) {
        if (entity.status === "current") table[`${entity.kind}:${entity.name}`] = entity.ref;
      }
    }
    return table;
  }, [live]);

  /// The readings this one replaces, as the rung's own action already says:
  /// what a re-read supersedes is the Cycle's word, never this file's.
  const supersedes = useMemo(() => {
    const rung = slot.ladder?.rungs.find((step) => step.step === "projection");
    const preset = rung?.actions?.find((offer) => offer.tool === "project_vision")?.preset;
    const parents = preset?.parents;
    return Array.isArray(parents) ? parents.filter((parent): parent is string => typeof parent === "string") : [];
  }, [slot]);

  /// The readings written before what was asked changed, in full, so the
  /// editor can come back with the words that were written.
  const outdated = useMemo(() => live.filter((view) => supersedes.includes(view.ref)), [live, supersedes]);

  const rowFor = (kind: string, name: string): Row => rows[`${kind}:${name}`] ?? blankRow(declarationOf(kind));
  const setRow = (kind: string, name: string, next: Row) => setRows((previous) => ({ ...previous, [`${kind}:${name}`]: next }));

  if (!projection) return null;
  if (!open) {
    // Reading again is editing what was written, not starting over: the
    // words the person wrote last time come back with the editor, and what
    // they already made is cited rather than asked for twice.
    const again = outdated[0];
    return (
      <button
        id="decompose-open"
        className="rung-action"
        data-slot={slot.slot}
        data-again={again ? "true" : "false"}
        onClick={() => {
          if (again && reasoning === "") setReasoning(again.projection.reasoning);
          setOpen(true);
        }}
      >
        {again ? "Read what was asked again" : "Read the intent yourself"}
      </button>
    );
  }

  const record = async () => {
    setError("");
    const sourceRefs = texts.map((text) => text.ref);
    if (sourceRefs.length === 0) {
      setError("Nothing is recorded about what was asked; say that first.");
      return;
    }
    const wanted: Record<string, unknown>[] = [];
    const citing: Record<string, unknown>[] = [];
    for (const tag of tags) {
      const key = `${tag.kind}:${tag.name}`;
      const already = cited[key];
      if (already) {
        citing.push({ kind: tag.kind, name: tag.name, ref: already });
        continue;
      }
      const declaration = declarationOf(tag.kind);
      if (!declaration) {
        setError(`Nothing here makes a ${tag.kind}; the kinds are ${sketches.map((sketch) => sketch.kind).join(", ")}.`);
        return;
      }
      const row = rowFor(tag.kind, tag.name);
      const fields: Record<string, unknown> = {};
      for (const field of declaration.fields ?? []) {
        fields[field.name] = fieldValue(row.fields[field.name] ?? "", field.default);
      }
      wanted.push({ kind: tag.kind, name: tag.name, within: row.within, intent: row.intent, fields });
    }
    setBusy(true);
    try {
      await callProjectTool(serverName, "project_vision", {
        source_refs: sourceRefs,
        lens_ref: projection.lens,
        reasoning,
        slot: slot.slot,
        sketches: wanted,
        declared: citing,
        parents: supersedes,
      });
      setOpen(false);
      setReasoning("");
      setRows({});
      onChanged();
    } catch (failure) {
      setError((failure as Error).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <section id="decompose" className="rung-form decompose" data-slot={slot.slot} data-lens={projection.lens}>
      <div className="k-eyebrow">{outdated.length > 0 ? "Read what was asked again" : "Read the intent"}</div>
      {outdated.length > 0 ? (
        <p id="decompose-outdated" className="decompose-outdated">
          What was asked has changed since this was read. Your words are below; what you already made
          is cited, not made again.
        </p>
      ) : null}
      <blockquote id="decompose-intent" className="decompose-intent">
        {texts.map((text) => (
          <p key={text.ref} className={text.text === null ? "k-muted" : undefined}>
            {text.text ?? "Reading what was asked…"}
          </p>
        ))}
      </blockquote>
      <label className="elicitation-field">
        <span>Your reading</span>
        <textarea
          id="decompose-reasoning"
          rows={6}
          value={reasoning}
          placeholder={`Say why the piece is made of what it is made of, and tag each thing as [[${sketches[0]?.kind ?? "kind"}:name]].`}
          onChange={(event) => setReasoning(event.target.value)}
        />
        <small>
          {sketches.map((sketch) => `${sketch.kind}: ${sketch.summary}`).join(" · ")}
        </small>
      </label>
      <ul id="decompose-declared" className="decompose-declared">
        {tags.map((tag) => {
          const key = `${tag.kind}:${tag.name}`;
          const already = cited[key];
          const declaration = declarationOf(tag.kind);
          const row = rowFor(tag.kind, tag.name);
          const parents = declaration?.within
            ? tags.filter((candidate) => candidate.kind === declaration.within).map((candidate) => candidate.name)
            : [];
          return (
            <li key={key} className="decompose-row" data-tag={key} data-kind={tag.kind} data-name={tag.name} data-state={already ? "cited" : declaration ? "new" : "unknown"}>
              <code>{key}</code>
              {already ? (
                <span className="k-muted">already there</span>
              ) : !declaration ? (
                <span className="bad">nothing here makes a {tag.kind}</span>
              ) : (
                <>
                  {declaration.within ? (
                    <label>
                      <span>in</span>
                      <select
                        data-field="within"
                        value={row.within}
                        onChange={(event) => setRow(tag.kind, tag.name, { ...row, within: event.target.value })}
                      >
                        <option value="">choose a {declaration.within}</option>
                        {parents.map((parent) => (
                          <option key={parent} value={parent}>
                            {parent}
                          </option>
                        ))}
                      </select>
                    </label>
                  ) : null}
                  <input
                    data-field="intent"
                    placeholder="what belongs here, in the words of what was asked"
                    value={row.intent}
                    onChange={(event) => setRow(tag.kind, tag.name, { ...row, intent: event.target.value })}
                  />
                  {(declaration.fields ?? []).map((field) => (
                    <label key={field.name} title={field.summary}>
                      <span>{field.name}</span>
                      <input
                        data-field={field.name}
                        value={row.fields[field.name] ?? ""}
                        onChange={(event) =>
                          setRow(tag.kind, tag.name, { ...row, fields: { ...row.fields, [field.name]: event.target.value } })
                        }
                      />
                    </label>
                  ))}
                </>
              )}
            </li>
          );
        })}
      </ul>
      <div id="decompose-error" className="bad">
        {error}
      </div>
      <div className="elicitation-actions">
        <button id="decompose-record" className="primary" type="button" disabled={busy || tags.length === 0} onClick={() => void record()}>
          {busy ? "Working…" : "Record the reading"}
        </button>
        <button id="decompose-cancel" type="button" onClick={() => setOpen(false)}>
          Cancel
        </button>
      </div>
    </section>
  );
}
