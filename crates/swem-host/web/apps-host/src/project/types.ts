// Envelope semantics only: exact record refs and the resource URIs the server
// declares. No domain schema is interpreted here; unknown fields pass through.

export type Ref = string;

/// One project as the list knows it. Most of it is known-later: a row exists
/// from the declaration alone, and what the server says of itself arrives
/// once that server has been opened. The host does not guess in between.
export interface ProjectSourceView {
  server_name: string;
  root_uri?: string;
  description?: string | null;
  /// The server's own word for where its records live; absent until asked.
  persistence?: string;
  /// `true` for a journal-backed project, `false` for one that keeps nothing
  /// or cannot be dialled, `null` until the server has been asked.
  available: boolean | null;
  connection_scope: string;
}

export interface ArtifactView {
  type_ref: string;
  media_type: string;
  content_digest: string;
  byte_length: number;
  /// The resource URI the server declares for the artifact's bytes.
  uri: string;
}

/// What the host records once it brought an artifact's bytes into its
/// content store: the descriptor that names them and serves them.
export interface ContentDescriptor {
  descriptor_id: string;
  content_digest: string;
  byte_length: number;
  media_type: string;
  name: string;
  source: string;
  producer: string;
  semantic_reference?: string | null;
  uri: string;
}

export interface PortView {
  name: string;
  type_ref: string;
  required: boolean;
}

export interface EffectView {
  interface: string;
  capability_class: string;
}

/// Whether a pin still says something true about the revision that carries
/// it. A stale pin keeps its claim and loses its citation.
export type PinState = "current" | "stale" | "unresolved_target" | "unresolved_record";

/// The fragment a selector recorded, as the Cycle read it. The selector
/// itself never reaches this client: its semantics belong to the artifact
/// type, and interpreting one here would be a second reader of a schema this
/// layer does not own.
export interface CitationView {
  selector_schema: string;
  quote?: string | null;
}

/// One grounding a revision pins, with its state against that revision.
export interface GroundedPin {
  ref: Ref;
  state: PinState;
  citation?: CitationView | null;
}

/// One unit of the model plane as the kernel reads it: its ports and the
/// effects it requires. `intent` is the author's one line, and `grounded_by`
/// is why it exists, derived by the Cycle against this revision's snapshot.
export interface NodeView {
  id: string;
  type_ref: string;
  semantic_payload_pointer: string;
  intent?: string;
  inputs: PortView[];
  outputs: PortView[];
  effects: EffectView[];
  grounded_by?: GroundedPin[];
}

/// The model plane of one revision: units, connections (`from.port -> to.port`),
/// roots (root id -> `node.port`) and the required inputs nothing feeds.
export interface ModelView {
  nodes: NodeView[];
  connections: string[];
  roots: Record<string, string>;
  unbound_inputs: string[];
}

export interface RevisionView {
  ref: Ref;
  uri: string;
  logical_id: string;
  parents: Ref[];
  grounding_refs: Ref[];
  /// Pins this revision carries that no rendered unit owns.
  grounded_elsewhere?: GroundedPin[];
  selection_refs: Ref[];
  model?: ModelView | null;
  model_error?: string | null;
  projection_refs?: Ref[];
  /// Unit ids of this revision no projection declares.
  unprojected?: string[];
  /// What this revision is made of, in the owning modules' words.
  units?: UnitView[];
}

/// One unit of what a revision is made of, as the kernel derives it from the
/// owning module: a track, a clip, a service node. An unfilled unit is a
/// sketch - it still needs what `requires` names.
export interface UnitView {
  id: string;
  pointer: string;
  kind: string;
  title: string;
  intent?: string;
  filled: boolean;
  requires?: string[];
}

/// One declared entity of a projection with its derived status: `current`,
/// `stale` (its revision was superseded) or `unresolved`.
export interface DeclaredView {
  kind: string;
  name: string;
  ref: string;
  status: string;
}

export interface ProjectionView {
  ref: Ref;
  projection: { source_set_ref: Ref; lens_ref: string; reasoning: string; parents?: Ref[] };
  declared: DeclaredView[];
  superseded_by?: Ref[];
  /// Intent texts the project holds that this reading never read: what was
  /// asked after it was written. The Cycle derives it; this layer only
  /// shows it and passes the reading it replaces back as `parents`.
  unread_sources?: Ref[];
}

export interface RequirementView {
  constraint_ref: Ref;
  predicate_contract_ref: string;
  subject_root: string;
  resolved_subject: string | null;
  status: string;
  reason: string;
  evidence_refs: Ref[];
  /// Why the requirement exists, as the constraint records it, classified by
  /// the Cycle against its record set.
  source_provenance?: { ref: Ref; kind: string; schema?: string | null }[];
  produced_by_tool: string;
}

export interface ObligationView {
  boundary: string;
  subject_ref: Ref;
  required_predicate: string;
  introduced_by: string;
  possible_discharge: string;
  blocking: boolean;
}

export interface ObligationsView {
  requirements: RequirementView[];
  obligations: ObligationView[];
  resolved_roots: Record<string, string>;
  acceptance: { decision: string; [key: string]: unknown } | null;
  acceptance_blocked_by: string | null;
  ready: boolean;
  next_tools: string[];
  evaluation_records: number;
}

export interface ExecutionView {
  roots: string[];
  result: { kind?: string; [key: string]: unknown } | null;
  produced_artifacts: Record<string, ArtifactView>;
  error: string | null;
}

export interface SelectionView {
  ref: Ref;
  uri: string;
  logical_id: string;
  selection: { model_revision_ref: Ref; roots: string[]; acceptance_policy_ref: Ref };
  supersedes: Ref[];
  superseded_by: Ref[];
  closures: { ref: Ref; [key: string]: unknown }[];
  executions: Record<string, ExecutionView>;
  obligations: ObligationsView | null;
  obligations_error: string | null;
}

export interface LineView {
  logical_id: string;
  revisions: RevisionView[];
  heads: Ref[];
  amendments: { ref: Ref; parent_ref: Ref; child_ref: Ref; introduced_by: string }[];
  amendment_decisions: { ref: Ref; allowed: boolean; authority_ref: string }[];
  selections: SelectionView[];
  branch_tips: Ref[];
  /// The ladder of this line alone, as the Cycle derives it.
  ladder?: LadderView | null;
}

/// One long operation as its start and end records tell it. `working` means
/// no end record has reached the journal yet: in progress, or the process
/// that ran it died; the envelope has no lease to tell them apart.
export interface OperationView {
  id: string;
  tool: string;
  status: string;
  started_ref?: Ref | null;
  ended_ref?: Ref | null;
  delivery_selection_ref?: Ref | null;
  revision_ref?: Ref | null;
  result_refs: Record<string, Ref>;
  error?: string | null;
}

/// One step of the product ladder, in the Cycle's own spelling.
export type LadderStep = "vision" | "projection" | "units" | "verification" | "delivery" | "effects";

/// What the records say about one rung. `attention` is not a failure: it
/// marks a rung holding something the owner has to look at.
export type RungState = "empty" | "present" | "attention" | "blocked" | "accepted";

/// One thing that can be done at a rung: the tool, what to call it, and the
/// arguments the Cycle already decided. The surface builds the form from
/// the tool's own input schema; it names no tool of its own.
export interface RungAction {
  tool: string;
  label: string;
  preset?: Record<string, unknown>;
}

export interface RungView {
  step: LadderStep;
  state: RungState;
  refs: Ref[];
  owed: string[];
  actions?: RungAction[];
  /// What this rung produced. A delivery that closed has something to show
  /// and nothing left to do, so this is the only thing such a rung says.
  produced?: ProducedView[];
  /// What the runs behind this rung have to say to a person, already in
  /// sentences. An accepted delivery is not always one to send: the run may
  /// know the master came out louder than the file holds while the
  /// acceptance says nothing, because nothing was violated.
  notices?: string[];
  /// The long operations of this step that have started and not ended, as
  /// the Cycle words them. The rung's own action is gone by then - a
  /// release that has chosen its roots is no longer a head waiting to be
  /// released - so the operation cannot be found through the actions.
  running?: RunningView[];
}

/// One thing this rung's step has in flight.
export interface RunningView {
  operation_id: string;
  text: string;
}

/// One thing a closed delivery produced: the root of the selection it came
/// out of, and - where that root resolved to a typed artifact the project
/// holds - what it is and how to ask the host for its bytes.
export interface ProducedView {
  root: string;
  /// The path a companion was written at beside the root's output
  /// (`stems/keys.wav`). Absent on the output itself.
  file?: string;
  ref?: Ref;
  /// The selection the closure stands under: which branch these bytes are
  /// read under when the page asks the host for them.
  selection: Ref;
  /// The model revision the delivery was selected over: what tells two
  /// releases of the same piece apart.
  revision: Ref;
  name?: string;
  type_ref?: string;
  media_type?: string;
  byte_length?: number;
  /// The content digest without its prefix: what the artifact door takes.
  digest?: string;
}

/// The one suggested next step. The Workbench renders it; it never derives
/// it - the Cycle does, and the agent's digest reads the same value.
export interface NextStep {
  step: LadderStep;
  text: string;
}

export interface LadderView {
  rungs: RungView[];
  next?: NextStep | null;
}

/// One slot: a named place for one module's work, and the workbench that
/// opens on it (`null` when this build has no such module).
export interface SlotView {
  slot: string;
  module: string;
  /// What its kind of work is called, in a person's words.
  title?: string;
  app_uri: string | null;
  requires?: string[];
  /// The tool that begins this slot's line of work, named by its module.
  door?: string;
  /// How an intent is decomposed into this slot's work, as its module
  /// declares it: the lens the reasoning is read through and the kinds of
  /// thing the reading may ask to be made. Absent when the module declares
  /// none, and then the rung stays an agent's.
  projection?: SlotProjectionView;
  /// The ladder of the line named after the slot, as the Cycle derives it.
  ladder?: LadderView | null;
}

/// One word a kind of sketch takes besides its name and its intent, with
/// what it is worth unless a person says otherwise.
export interface SketchFieldView {
  name: string;
  summary: string;
  default?: unknown;
}

/// One kind of thing a reading may ask to be made: what it is called, what
/// one is, what it sits inside, and the words it takes.
export interface SketchKindView {
  kind: string;
  module: string;
  summary: string;
  within?: string;
  fields?: SketchFieldView[];
}

export interface SlotProjectionView {
  lens: string;
  sketches?: SketchKindView[];
}

export interface SlotKindView {
  /// What this kind of work is called, in a person's words.
  title?: string;
  summary?: string;
  module: string;
  app_uri: string;
}

export interface SlotsView {
  heads?: Ref[];
  slots: SlotView[];
  kinds: SlotKindView[];
}

/// One asset of the project: the typed artifact the ontology holds, with the
/// name a person gave it and the records that use it.
export interface AssetView {
  ref: Ref;
  digest: string;
  name?: string;
  note?: string;
  type_ref: string;
  media_type: string;
  byte_length: number;
  uri: string;
  produced: boolean;
  /// Absent when nothing uses it yet: the envelope leaves empty lists out.
  used_by?: Ref[];
}

export interface AssetsView {
  heads?: Ref[];
  assets: AssetView[];
}

export interface Envelope {
  persistence: string;
  record_set_digest: string;
  /// What the project is made of. Older servers omit it; the tab strip then
  /// shows every workbench the server publishes, as before.
  slots?: SlotsView;
  assets?: AssetsView;
  /// The ladder as the Cycle derives it. Older servers omit it; the axis
  /// then says the server does not publish one rather than inventing rungs.
  ladder?: LadderView;
  sources: {
    source_sets: { ref: Ref; sources: Ref[] }[];
    groundings: { ref: Ref; grounding: { source_ref: Ref; target_pointer: string; interpretation: string } }[];
    /// The record above is dereferenced by ref for its claim and its source;
    /// which unit it belongs to is the Cycle's derivation, never a match on
    /// `target_pointer` performed here.
    artifacts: Record<Ref, ArtifactView>;
    projections?: ProjectionView[];
  };
  logical_ids: LineView[];
  unanchored: Record<string, Ref[]>;
  /// Long operations over the whole project, by operation id.
  operations?: OperationView[];
}

export interface AgentContextBinding {
  server_name: string;
  revision_ref: Ref;
  selection_ref?: Ref | null;
  uri: string;
}

/// What the operator is looking at. Changing it never touches agent context.
export interface Viewing {
  server_name: string | null;
  logical_id: string | null;
  revision_ref: Ref | null;
  selection_ref: Ref | null;
}

/// One domain workbench a project publishes as an MCP App resource.
export interface AppView {
  uri: string;
  description?: string;
}

/// A tool the project's server declares, as the host discovered it: the
/// schema the form is built from and whether a person may call it.
export interface ToolView {
  name: string;
  title?: string;
  description?: string;
  input_schema: unknown;
  visibility: string[];
}
