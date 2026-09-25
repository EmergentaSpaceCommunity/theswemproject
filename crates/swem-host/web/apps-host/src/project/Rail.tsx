// The cycle as a rail: where the work stands, what is owed, what can be
// done here - and, per slot, the door that begins its line and the
// workbench that opens on it. Every value shown is derived by the Cycle and
// carried in the project envelope - the same derivation the agent reads
// through `read_context`. Nothing is computed from the records a second time
// here, because a second derivation would be a second answer to "where does
// the work stand", and the two would drift.

import { useState } from "react";

import { Decompose } from "./Decompose";
import { marked, outputName, readable, unmarked } from "./sentences";
import { ToolForm, formShapeProblem } from "./ToolForm";
import { toolFor } from "./slots";
import { materializeArtifact } from "./api";
import type { ContentDescriptor, Envelope, LadderStep, LadderView, ProducedView, RungAction, RungView, SlotView, ToolView, UnitView } from "./types";

/// The owner-facing name of each step, in ladder order. The Cycle sends the
/// step's identifier; the wording belongs to the surface.
export const TITLES: Record<LadderStep, string> = {
  vision: "What was asked",
  projection: "How it was read",
  units: "What it is made of",
  verification: "What was checked",
  delivery: "What was produced",
  effects: "What touches the outside",
};

/// How big a thing is, where a person reads sizes.
function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/// The file name a person would expect for one of these. A companion was
/// written at a path and keeps that path's own name; a root's output has no
/// name but the domain's word for the root. The digest is a fine name for a
/// store and a poor one for a download folder.
export function fileName(one: ProducedView): string {
  if (one.file !== undefined) return one.file.slice(one.file.lastIndexOf("/") + 1);
  const stem = (one.name ?? outputName(one.root)).replace(/[^a-z0-9]+/gi, "-").replace(/^-|-$/g, "").toLowerCase();
  const suffix = EXTENSION[one.media_type ?? ""] ?? "";
  return `${stem || "output"}${suffix}`;
}

/// What a row is called on the rung. A companion is called by its path,
/// because that is what a person sees in the folder and what tells two stems
/// apart; only its extension is dropped, which says nothing the media type
/// beside it does not.
function rowName(one: ProducedView): string {
  if (one.name !== undefined) return one.name;
  if (one.file === undefined) return outputName(one.root);
  const dot = one.file.lastIndexOf(".");
  const slash = one.file.lastIndexOf("/");
  return dot > slash ? one.file.slice(0, dot) : one.file;
}

/// Only what a release actually produces. A table of every media type would
/// be this surface guessing at domains it does not have.
const EXTENSION: Record<string, string> = {
  "audio/wav": ".wav",
  "audio/x-wav": ".wav",
  "audio/mpeg": ".mp3",
  "audio/flac": ".flac",
  "application/json": ".json",
  "text/plain": ".txt",
};

/// What identifies a row among a delivery's rows: a companion by its path, a
/// root's output by the root. Two roots cannot share a path and a root has
/// one output, so this is unique without a digest in it.
function rowKey(one: ProducedView): string {
  return one.file === undefined ? one.root : `${one.root}/${one.file}`;
}

/// One thing a delivery produced: what it is, and - once the person asks for
/// the bytes - the player and the download they came for.
///
/// The bytes are fetched when the person presses, never on their own. An
/// earlier draft fetched as soon as the row appeared, which put a call to the
/// project's server in the same moment as the release that had just been
/// accepted: the engine was still writing its stems, and one of them was
/// cancelled under it, so a role that had played out loud a second earlier
/// rendered silent. A rung reporting what happened must not reach for the
/// bytes while the thing it reports is still landing.
function ProducedRow({ one, serverName }: { one: ProducedView; serverName: string | null }) {
  const [descriptor, setDescriptor] = useState<ContentDescriptor | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const digest = one.digest;
  const audible = (one.media_type ?? "").startsWith("audio/");
  const bring = async () => {
    if (!serverName || !digest) return;
    setBusy(true);
    setError(null);
    try {
      const result = await materializeArtifact<ContentDescriptor>(serverName, digest, one.selection);
      setDescriptor(result.descriptor);
    } catch (failure: unknown) {
      setError(String(failure));
    } finally {
      setBusy(false);
    }
  };
  const served = descriptor ? `/api/content/${descriptor.descriptor_id}` : null;
  return (
    <li className="produced-row" data-root={one.root} data-file={one.file ?? ""} data-digest={digest ?? ""} data-descriptor-id={descriptor?.descriptor_id ?? ""}>
      <strong className="produced-name">{rowName(one)}</strong>
      <span className="k-caption k-muted">
        {one.media_type ? `${one.media_type}` : "no bytes recorded"}
        {one.byte_length === undefined ? "" : ` \u00b7 ${size(one.byte_length)}`}
      </span>
      {served ? (
        <>
          <a className="produced-download" data-root={one.root} data-file={one.file ?? ""} href={`${served}?download=1`} download={fileName(one)}>
            Save as {fileName(one)}
          </a>
          {audible ? <audio className="produced-play" data-root={one.root} data-file={one.file ?? ""} controls preload="metadata" src={served} /> : null}
        </>
      ) : (
        <button
          className="produced-bring"
          data-root={one.root}
          data-file={one.file ?? ""}
          type="button"
          disabled={busy || !serverName || !digest}
          onClick={() => {
            void bring();
          }}
        >
          {busy ? "Fetching\u2026" : audible ? "Listen" : "Get the file"}
        </button>
      )}
      {error ? <span className="bad k-caption">{error}</span> : null}
    </li>
  );
}

/// What a delivery produced, where the person who made it is looking. A rung
/// that has closed has nothing left to do, and this is the whole of what it
/// has left to say.
function Produced({ produced, serverName }: { produced: ProducedView[]; serverName: string | null }) {
  // Which revision this came out of, said once. A person who comes back to
  // two releases of the same piece has nothing else to tell them apart.
  const revisions = [...new Set(produced.map((one) => one.revision))];
  return (
    <>
      <p className="rung-produced-from k-caption k-muted" data-revision={revisions.join(" ")}>
        from {revisions.length === 1 ? "revision" : "revisions"} {revisions.map((one) => shortRef(one)).join(", ")}
      </p>
      <ul className="rung-produced">
        {produced.map((one) => (
          <ProducedRow key={rowKey(one)} one={one} serverName={serverName} />
        ))}
      </ul>
    </>
  );
}

/// A ref as a person reads it back: enough to tell two apart, not enough to
/// be mistaken for something to type.
function shortRef(value: string): string {
  return value.startsWith("sha256:") ? value.slice(7, 19) : value;
}

/// One line saying what a state means, so a rung is legible without a legend.
const MEANING: Record<string, string> = {
  empty: "nothing recorded",
  present: "recorded",
  attention: "needs a look",
  blocked: "owes an obligation",
  accepted: "accepted",
};

/// What a person does next at each step, in their words; the Cycle's own
/// sentence (tool names and refs) stays as the title for whoever wants it.
const NEXT: Record<LadderStep, string> = {
  vision: "say what you want made",
  projection: "read the intent through a lens",
  units: "begin the work in a slot",
  verification: "check what was made",
  delivery: "say what a delivery must satisfy, then choose what to produce",
  effects: "detach the outputs",
};

/// The next step as a person reads it.
export function nextPhrase(ladder: LadderView): string {
  const next = ladder.next;
  if (!next) return "Nothing is owed by the records.";
  // A units rung that needs a look has work begun and still owed: the
  // person fills it where the work is, not at the door.
  const owedUnits = next.step === "units" && ladder.rungs.some((rung) => rung.step === "units" && rung.state === "attention");
  const phrase = owedUnits ? "fill what the work still owes" : (NEXT[next.step] ?? next.text);
  return `Next: ${TITLES[next.step] ?? next.step} - ${phrase}`;
}

/// Said wherever a slot's rail has to send a person to the project's rail for
/// what was asked: the reason the workbench button is not yet pressable, and
/// the caption over that rail's next step. One spelling, used twice.
const ASK_FIRST = "Record what was asked first; the workbench opens on it.";

/// The same ladder without the rung that belongs to the project, for a rail
/// that is not the project's.
///
/// `vision` is derived in `swem-cycle` from the project's own source artifacts
/// with no reference to the line being read (`ladder.rs`), so the Cycle hands
/// every rail the identical rung - same state, same refs - and it always
/// carries `put_text_source`. Drawn on a slot's rail that reads as an intent
/// belonging to the slot, and pressing it records the project's. What was
/// asked is the project's, and the project's rail still shows it.
///
/// The records are untouched: the envelope still carries six rungs per line,
/// and `swem-cycle`'s own test that a narrowed ladder agrees with the
/// project's keeps holding. This stops one record being repeated once per
/// slot on screen; it does not change what a ladder is.
export function withoutTheProjectsOwnRungs(ladder: LadderView): LadderView {
  return { ...ladder, rungs: ladder.rungs.filter((rung) => rung.step !== "vision") };
}

/// One sentence of the Cycle, with what it marked as a name rendered as one
/// rather than printed with its marks showing.
function Said({ text }: { text: string }) {
  return (
    <>
      {marked(text).map((part, at) =>
        part.name ? (
          // eslint-disable-next-line react/no-array-index-key -- the parts of one sentence have no other identity
          <code key={at} className="said-name">
            {part.text}
          </code>
        ) : (
          <span key={at}>{part.text}</span>
        ),
      )}
    </>
  );
}

export interface Acting {
  serverName: string;
  tools: ToolView[];
  onChanged: () => void;
}

/// One action of a rung: a form when the tool's schema is flat enough for a
/// person, otherwise an honest note that this step is an agent's.
function Action({ action, acting }: { action: RungAction; acting: Acting }) {
  const [open, setOpen] = useState(false);
  const tool = toolFor(acting.tools, action.tool);
  const problem = tool ? formShapeProblem(tool, action) : "the server does not offer this tool to a person";
  if (!tool || problem) {
    // The tool's name is for whoever hovers; the line reads as a step.
    return (
      <p className="rung-agent-step k-muted" data-action-tool={action.tool} title={`${action.tool}${problem ? `: ${problem}` : ""}`}>
        <Said text={action.label} /> - an agent does this
      </p>
    );
  }
  if (!open) {
    return (
      <button className="rung-action" data-action-tool={action.tool} onClick={() => setOpen(true)}>
        {/* Through the same reader as every other line the Cycle writes. This
            was the one that did not, so a person read "Release `music`" with
            the marks in it while the rung above rendered them as code. */}
        <Said text={action.label} />
      </button>
    );
  }
  return (
    <ToolForm
      serverName={acting.serverName}
      action={action}
      tool={tool}
      onDone={() => {
        setOpen(false);
        acting.onChanged();
      }}
      onCancel={() => setOpen(false)}
    />
  );
}

function Rung({ rung, isNext, acting, extra, claimed, zoom }: { rung: RungView; isNext: boolean; acting: Acting | null; extra?: React.ReactNode; claimed?: string[]; zoom?: boolean }) {
  // The row opens a step where its attention is - the next step, or one that
  // needs a look or owes an obligation; a settled step is its chip, until
  // someone asks for it. A settled rung is not a closed one: a project's
  // intent grows as it goes, and the Cycle keeps offering the step, so a
  // person has to be able to take it after it first went quiet.
  const [asked, setAsked] = useState(false);
  const produced = rung.produced ?? [];
  // A notice is the reason a person would not send what they just made, and
  // the rung carrying it reads `accepted`, which is a settled state and
  // stays shut. So a rung with something to say opens itself: a caution
  // behind a click is a caution the person who walks away never reads.
  const notices = rung.notices ?? [];
  // A step a person set going is a step with something to say, so it says it
  // without being asked: a rung that has just been pressed and looks settled
  // is exactly the rung nobody thinks to click. Which operations are this
  // rung's, and what to call them, is the Cycle's answer - the rung's own
  // action is gone by the time there is anything to say.
  const running = rung.running ?? [];
  const open =
    isNext ||
    rung.state === "attention" ||
    rung.state === "blocked" ||
    notices.length > 0 ||
    running.length > 0 ||
    asked;
  // A step opens where it has something for the person: an action to take, or
  // something it produced to look at. Before this the second half was missing,
  // so the rung whose job is to report became unopenable at the exact moment
  // it finally had something to report.
  const offers =
    (Boolean(acting) && (rung.actions?.length ?? 0) > 0) ||
    produced.length > 0 ||
    notices.length > 0 ||
    running.length > 0;
  // On a slot's card the steps are not a ladder - the project above has the
  // one ladder - but the places this slot has something for the person: what
  // it owes, what it is made of, what it can do, what it produced. A step
  // with nothing of the kind is not drawn there at all.
  if (zoom && !offers && !extra && rung.owed.length === 0) return null;
  return (
    <li
      className="rung"
      data-ladder-step={rung.step}
      data-state={rung.state}
      data-open={open ? "true" : "false"}
      data-running={running.length > 0 ? "true" : undefined}
      data-next={isNext ? "true" : undefined}
      data-zoom={zoom ? "true" : undefined}
      aria-current={isNext ? "step" : undefined}
    >
      {/* The chip: the step's name, its state as the chip's colour and its
          meaning as the title a pointer reveals. It opens the step where the
          step has something to offer. */}
      {offers ? (
        <button
          type="button"
          className="rung-title"
          aria-expanded={open}
          title={`${MEANING[rung.state] ?? rung.state}${rung.refs.length > 0 ? ` · ${rung.refs.length}` : ""}`}
          onClick={() => setAsked((was) => !was)}
        >
          {TITLES[rung.step] ?? rung.step}
        </button>
      ) : (
        <span
          className="rung-title"
          title={`${MEANING[rung.state] ?? rung.state}${rung.refs.length > 0 ? ` · ${rung.refs.length}` : ""}`}
        >
          {TITLES[rung.step] ?? rung.step}
        </span>
      )}
      {/* What is going on right now, above everything the rung has already
          got: a person reading the step wants first to know whether it is
          still happening. Not behind `open` - a running step says so whether
          or not anyone opened it. */}
      {running.length > 0 ? (
        <ul className="rung-running" aria-label="Running now">
          {running.map((one) => (
            <li key={one.operation_id} className="rung-running-row" data-operation-id={one.operation_id}>
              <Said text={one.text} />
            </li>
          ))}
        </ul>
      ) : null}
      {/* Above the files, because it is why a person might not want them. The
          run wrote the sentence; nothing here rewords it. */}
      {open && notices.length > 0 ? (
        <ul className="rung-notices">
          {notices.map((notice) => (
            <li key={notice} className="rung-notice">
              <Said text={notice} />
            </li>
          ))}
        </ul>
      ) : null}
      {open && produced.length > 0 ? <Produced produced={produced} serverName={acting?.serverName ?? null} /> : null}
      {open && rung.owed.length > 0 ? (
        <ul className="rung-owed">
          {rung.owed.map((owed) => (
            <li key={owed} title={unmarked(readable(owed))}>
              <Said text={readable(owed)} />
            </li>
          ))}
        </ul>
      ) : null}
      {(open && acting && rung.actions && rung.actions.length > 0) || extra ? (
        <div className="rung-actions">
          {open && acting && rung.actions
            ? rung.actions
                .filter((action) => !(claimed ?? []).includes(action.tool))
                .map((action) => <Action key={action.tool} action={action} acting={acting} />)
            : null}
          {extra}
        </div>
      ) : null}
    </li>
  );
}

/// The rungs of one ladder and the one step it points at. `perRung` lets a
/// caller put its own control on a rung - the slot rail puts the workbench
/// on "what it is made of" - and `claimed` names the tools that caller
/// renders itself, so a rung does not also offer them as "an agent does
/// this" beside the control that does them.
export function Rungs({ ladder, acting, perRung, nextId, plain, nextCaption, claimed, zoom }: { ladder: LadderView; acting: Acting | null; perRung?: (step: LadderStep) => React.ReactNode; nextId?: string; plain?: boolean; nextCaption?: string; claimed?: string[]; zoom?: boolean }) {
  const next = ladder.next ?? null;
  return (
    <>
      <ol className={zoom ? "rungs rungs-zoom" : "rungs"}>
        {ladder.rungs.map((rung) => (
          <Rung key={rung.step} rung={rung} isNext={next?.step === rung.step} acting={acting} extra={perRung?.(rung.step)} claimed={claimed} zoom={zoom} />
        ))}
      </ol>
      {next && nextCaption ? <p className="k-caption k-muted">{nextCaption}</p> : null}
      <p id={nextId} className="k-muted rail-next" data-next-step={next?.step} title={next ? unmarked(next.text) : undefined}>
        {next ? (
          <Said text={plain ? next.text : nextPhrase(ladder)} />
        ) : (
          "Nothing is owed by the records. Detach the outputs, or amend the model."
        )}
      </p>
    </>
  );
}

/// The project's own rail: what was asked and how it was read, over every
/// slot. Its identifier is the one the surface has always carried.
export function ProjectRail({ ladder, acting }: { ladder: LadderView | null | undefined; acting: Acting | null }) {
  if (!ladder) {
    return (
      <section id="cycle-axis" aria-label="Cycle">
        <h3>Cycle</h3>
        <p className="k-muted">This project server publishes no ladder; its records are shown below as they are.</p>
      </section>
    );
  }
  return (
    <section id="cycle-axis" className="rail" aria-label="Cycle">
      <header className="rail-head">
        <strong>The project</strong>
      </header>
      {/* The project rail keeps the Cycle sentence as it is: this is where a
          person sees exactly what an agent reads. Which is worth saying out
          loud: without the line below, a sentence naming tools and resource
          addresses reads as an instruction to the person, and the person
          cannot carry it out. */}
      <Rungs
        ladder={ladder}
        acting={acting}
        nextId="cycle-axis-next"
        plain
        nextCaption="What an agent reading this project is told to do next:"
      />
    </section>
  );
}

/// Whether the intent is recorded, which is what the gate opens every door
/// on - a workbench included, since its tools go through the same gate.
function intentRecorded(ladder: LadderView | null | undefined): boolean {
  return (ladder?.rungs.find((rung) => rung.step === "vision")?.state ?? "empty") !== "empty";
}

/// What a line is made of, as the kernel derived it from the owning module:
/// a track or a service node, a clip, a sketch. A sketch is a clip with no
/// audio bound yet - shown dashed, with what it still needs - which is the
/// same "missing" the ladder owes and the DAW draws. This is the workbench's
/// projection, before the workbench is opened.
function UnitList({ units }: { units: UnitView[] }) {
  if (units.length === 0) return null;
  return (
    <ul className="slot-units" aria-label="What it is made of">
      {units.map((unit) => (
        <li
          key={`${unit.kind}:${unit.id}`}
          className="slot-unit"
          data-kind={unit.kind}
          data-unit={unit.id}
          data-filled={unit.filled ? "true" : "false"}
        >
          <span className="slot-unit-title">{unit.title}</span>
          <span className="slot-unit-kind k-muted">{unit.kind}</span>
          {unit.intent ? <span className="slot-unit-intent">{unit.intent}</span> : null}
          {!unit.filled ? (
            <span className="slot-unit-missing" data-missing="true">
              needs {(unit.requires ?? ["more"]).join(", ")}
            </span>
          ) : null}
        </li>
      ))}
    </ul>
  );
}

/// One slot's rail: its six rungs as the Cycle derives them for the line
/// named after the slot, the door on "what it is made of" while the line
/// does not exist, and the workbench that opens on the slot once the intent
/// is recorded - a domain workbench is a projection of the line, never a
/// separate app.
export function SlotRail({ slot, units, projectLadder, envelope, acting, onOpen }: { slot: SlotView; units: UnitView[]; projectLadder: LadderView | null | undefined; envelope: Envelope; acting: Acting | null; onOpen: (slot: string) => void }) {
  const ladder = slot.ladder ?? null;
  // The intent's own type, as the vision rung's action declares it: what an
  // intent text is recorded as is the Cycle's word, never this file's.
  const intentTypeRef =
    (projectLadder?.rungs
      .find((rung) => rung.step === "vision")
      ?.actions?.find((action) => action.preset?.type_ref !== undefined)?.preset?.type_ref as string | undefined) ?? null;
  // A person reads the intent themselves only where the slot's module says
  // how its work is read out of one, and only once something was asked.
  const decompose =
    slot.projection && acting && intentRecorded(projectLadder) ? (
      <Decompose
        serverName={acting.serverName}
        slot={slot}
        envelope={envelope}
        intentTypeRef={intentTypeRef}
        onChanged={acting.onChanged}
      />
    ) : null;
  const begun = (ladder?.rungs.find((rung) => rung.step === "units")?.state ?? "empty") !== "empty";
  const openable = Boolean(slot.app_uri) && intentRecorded(projectLadder);
  const why = !slot.app_uri
    ? `${slot.title || slot.module}: not in this build`
    : !intentRecorded(projectLadder)
      ? ASK_FIRST
      : begun
        ? `Open the ${slot.title || slot.module} workbench on ${slot.slot}`
        : `Open the ${slot.title || slot.module} workbench on ${slot.slot}; it begins the work there`;
  const workbench = (
    <button
      className="open-workbench"
      data-slot={slot.slot}
      data-app-uri={slot.app_uri ?? undefined}
      data-begun={begun ? "true" : "false"}
      disabled={!openable}
      title={why}
      onClick={() => onOpen(slot.slot)}
    >
      Open {slot.title || slot.module}
    </button>
  );
  return (
    <section className="slot-rail rail" data-slot={slot.slot} data-module={slot.module} data-begun={begun ? "true" : "false"} aria-label={`Slot ${slot.slot}`}>
      <header className="rail-head slot-rail-head">
        <strong>{slot.slot}</strong>
        <span className="k-muted">
          {slot.title || slot.module}
          {slot.requires && slot.requires.length > 0 ? ` · needs ${slot.requires.join(", ")}` : ""}
        </span>
        {ladder ? <span className="rail-open">{workbench}</span> : null}
      </header>
      {ladder ? (
        // A slot is a zoom into the project, not a second ladder: the one
        // ladder is the project's, above. What the Cycle derives for this
        // slot's line is drawn here only where the slot has something for the
        // person - what it owes, what it is made of, what it can do, what it
        // produced - as sections under the slot, never as a row of steps.
        <Rungs
          ladder={withoutTheProjectsOwnRungs(ladder)}
          acting={acting}
          zoom
          claimed={decompose ? ["project_vision"] : undefined}
          // The rung is gone from this rail but the records still say it is
          // the next step, so say where that step is taken rather than point
          // at nothing.
          nextCaption={ladder.next?.step === "vision" ? ASK_FIRST : undefined}
          perRung={(step) => (step === "units" ? <UnitList units={units} /> : step === "projection" ? decompose : null)}
        />
      ) : (
        <p className="k-muted">This project server publishes no rail for the slot.</p>
      )}
    </section>
  );
}

/// The strip over an open workbench: the slot's rungs as chips, its next
/// step, and the way back. The workbench is a projection of this rail; the
/// rail never leaves the screen.
export function RailStrip({ slot, onBack }: { slot: SlotView; onBack: () => void }) {
  const ladder = slot.ladder ?? null;
  return (
    <nav id="rail-strip" className="rail-strip" data-slot={slot.slot} aria-label={`Rail of ${slot.slot}`}>
      <button id="rail-back" onClick={onBack}>
        ← Project
      </button>
      <strong>{slot.slot}</strong>
      <span className="k-muted">{slot.title || slot.module}</span>
      {/* No row of steps here either: the workbench is a zoom into the
          project, and the one ladder is the project's. What the slot owes
          next is worth a line; the steps are not. */}
      {ladder?.next ? <span id="rail-strip-next" className="k-muted rail-strip-next" title={ladder.next.text}><Said text={nextPhrase(ladder)} /></span> : null}
    </nav>
  );
}
