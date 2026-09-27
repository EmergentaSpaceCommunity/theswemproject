// The Agent space's state and every loop that feeds it, with no DOM in it.
//
// What this replaces: 924 lines of script in `shell.html` that bound 49 ids
// React had rendered empty, kept its state in module-level `let`s, and wrote
// whatever the host sent - a phase object, a terminal payload - straight
// into text nodes with `JSON.stringify`. That is where the red JSON the owner
// saw came from.
//
// The store owns one connection at a time. Every long-poll loop carries the
// generation it was started under and stops the moment the generation moves,
// which is how a second "Start session" cannot be fed by the first one's
// events. `fetch` is injected so a test drives the whole thing with a fake
// host and asks what a person would see.

import { emptyConversation, reduceEvent, type Conversation, type SurfaceEvent } from "./events.ts";

export const SURFACE_ID = "workbench-browser";

/// Where this browser keeps the name the person writes under.
const WRITING_AS_KEY = "swem.writing-as";

export interface Profile {
  profile_id: string;
  agent_id: string;
  /// How many times this profile has been amended; sent back when amending
  /// so two surfaces cannot overwrite one another silently.
  revision?: number;
  workspace?: string;
  agent_home?: string;
  attachments?: { profile_id: string; server_name: string; transport: string }[];
  /// Which of the host's permission choices answers this agent's requests.
  permission_profile_id?: string;
  /// Where the host runs this profile's agent.
  environment_profile_id?: string;
  /// The setup a person gave it: a model provider by id, a model, a role,
  /// skills. Each absent when never set.
  model_provider?: string | null;
  model?: string | null;
  role?: string;
  agent_skills?: AgentSkill[];
}

/// One skill a person wrote for their agent.
export interface AgentSkill {
  name: string;
  description: string;
  body: string;
}

/// The setup a form edits whole and sends back whole.
export interface AgentSetup {
  model_provider: string | null;
  model: string | null;
  role: string;
  agent_skills: AgentSkill[];
}

/// A place a model is served from, as the host lists it: shipped with the
/// product or declared by the person, with the kind of key it takes.
export interface ModelProviderView {
  id: string;
  name: string;
  base_url?: string | null;
  key_type: string;
  models: { id: string; name: string }[];
  origin: "built_in" | "declared";
  /// The variable the key goes under, from the secret catalogue.
  key_env?: string | null;
  key_label: string;
}

/// What a person fills in to declare a model provider.
export interface DeclareModelProvider {
  id: string;
  name: string;
  base_url?: string;
  key_type: string;
  models: { id: string; name: string }[];
}

/// One way an agent's permission requests can be answered, as the host
/// describes it. The words are the host's, so the page never invents a
/// boundary the host does not enforce.
export interface PermissionProfileOption {
  permission_profile_id: string;
  name: string;
  summary: string;
}

/// One place this host can run an agent, and the words the host uses to
/// describe it. Today the host offers one; the page reads the list rather than
/// naming it, so the second one appears here without the page changing.
export interface EnvironmentProfileOption {
  environment_profile_id: string;
  name: string;
  summary: string;
}

/// One MCP server an agent here can attach: one the product declared, or one
/// the person declared or installed. Never carries a secret - only the names of
/// the variables and headers a declaration sets.
/// One block of what an App said the model should know: the two kinds this
/// host declares it takes.
export type ModelContextBlock =
  | { type: "text"; text: string }
  | { type: "resource_link"; uri: string; name: string; mimeType?: string; description?: string };

/// What the agent is given with every next turn until it is let go of, and
/// the server whose App said it.
export interface ModelContext {
  server_name: string;
  content: ModelContextBlock[];
}

/// The blocks of an update this host takes, in the shape the host keeps
/// them. Anything else an App sent is left out: the host said which kinds it
/// takes when the App asked.
export function contextBlocks(content: unknown): ModelContextBlock[] {
  if (!Array.isArray(content)) return [];
  const taken: ModelContextBlock[] = [];
  for (const block of content) {
    if (!block || typeof block !== "object") continue;
    const one = block as Record<string, unknown>;
    if (one.type === "text" && typeof one.text === "string" && one.text.trim()) {
      taken.push({ type: "text", text: one.text });
    } else if (one.type === "resource_link" && typeof one.uri === "string" && typeof one.name === "string") {
      taken.push({
        type: "resource_link",
        uri: one.uri,
        name: one.name,
        ...(typeof one.mimeType === "string" ? { mimeType: one.mimeType } : {}),
        ...(typeof one.description === "string" ? { description: one.description } : {}),
      });
    }
  }
  return taken;
}

export interface McpServerView {
  name: string;
  transport: string;
  origin: "product" | "catalogue";
  command?: string;
  args?: string[];
  env_names?: string[];
  url?: string;
  header_names?: string[];
}

/// What a person fills in to declare a server.
export interface DeclareMcpServer {
  name: string;
  transport: string;
  command?: string;
  args?: string[];
  env?: { name: string; value: string }[];
  url?: string;
  headers?: { name: string; value: string }[];
}

/// One terminal running in a profile's environment.
export interface TerminalView {
  terminal_id: string;
  profile_id: string;
  command: string;
  args?: string[];
  /// `person` or `agent`: a terminal the agent opened over ACP is one of
  /// these too, and a person needs to tell them apart.
  opened_by: string;
  cols: number;
  rows: number;
  exit?: string;
}

export interface AgentOption {
  agent_id: string;
  name: string;
  readiness: string;
  available: boolean;
}

export interface Onboarding {
  enabled: boolean;
  agents: AgentOption[];
}

export interface AuthVariable {
  name: string;
  label?: string;
  secret?: boolean;
  optional?: boolean;
}

/// One advertised method, as the agent sent it. `type` is absent for a
/// plain method (authenticate by id, nothing to ask), `env_var` for one that
/// needs values the person types, `terminal` for one that needs a login run
/// outside this page.
export interface AuthMethod {
  type?: "env_var" | "terminal";
  id: string;
  name: string;
  description?: string;
  vars?: AuthVariable[];
  link?: string;
  args?: string[];
}

export interface Handshake {
  agent_name: string | null;
  agent_version: string | null;
  readiness: string;
  auth_methods: AuthMethod[];
}

export interface Phase {
  phase: "connecting" | "idle" | "prompting" | "finished";
  session_id?: string;
  turn_index?: number;
}

/// Where a permission question's words came from: the agent reported them,
/// or the host is about to carry out exactly what it wrote.
export type PermissionProvenance = "correlated_agent_report" | "uncorrelated_agent_report" | "host_callback";

export interface PermissionRequest {
  sequence: number;
  tool_call: { title?: string; toolCallId?: string };
  options: { optionId: string; kind: string; name: string }[];
  provenance?: PermissionProvenance;
}

export type StructuredPhase = "acp_form" | "acp_url" | "launch" | "elicitation";

export interface StructuredInteraction {
  phase: StructuredPhase;
  context: string;
  message: string;
  schema: unknown;
  sequence?: number;
  url?: string;
  serverName?: string;
  tool?: { name: string; description?: string; input_schema: unknown };
  interactionId?: string;
}

export interface AppTool {
  name: string;
  title?: string;
  description?: string;
  visibility: string[];
  resource_uri?: string;
  input_schema: unknown;
}

export interface AppAttachment {
  server_name: string;
  connection_scope: string;
  apps: { uri: string }[];
  tools: AppTool[];
}

export interface OpenedApp {
  app_id: string;
  server_name: string;
  uri: string;
  permissions: unknown;
}

/// One kind of secret the host knows how to inject (mirrors `SECRET_TYPES`).
export interface SecretType {
  type_id: string;
  label: string;
  env_var: string | null;
  hint: string;
}

/// A secret in the vault: its name and kind, never its value.
export interface SecretEntry {
  name: string;
  type_id: string;
  label: string;
}

export interface ProfileSecrets {
  types: SecretType[];
  secrets: SecretEntry[];
}

/// One session of this agent, as a person picks it from a list.
export interface SessionSummary {
  route_id: string;
  agent_id: string;
  last_sequence?: number;
  last_kind?: string;
  events: number;
  /// The first thing said in this conversation, which is what a person
  /// recognises it by. Absent on a lane nobody has written to yet.
  opened_with?: string;
}

/// A standing instruction a clock runs.
export interface Schedule {
  schedule_id: string;
  profile_id: string;
  say: string;
  every_minutes: number;
  enabled: boolean;
  last_claimed_ms?: number;
  route_id?: string;
  last_outcome?: string;
}

/// One file in the agent's inbox or outbox.
export interface HandedFile {
  area: string;
  name: string;
  byte_length: number;
}

export interface NewSecret {
  type_id: string;
  label: string;
  name?: string;
  value: string;
}

/// One value an agent's select option offers.
export interface ConfigChoice {
  value: string;
  name: string;
}

/// One thing the agent lets a session configure - its model, its mode, a
/// thought level - as ACP `configOptions` carries it, flattened: groups are
/// folded into one list of choices.
export interface ConfigOption {
  id: string;
  name: string;
  category: string | null;
  kind: "select" | "boolean";
  currentValue: string | boolean;
  choices: ConfigChoice[];
}

/// What the host holds of the agent's session configuration right now.
export interface SessionOptions {
  options: ConfigOption[];
  /// For an agent that predates config options: its modes.
  modes: { current: string; available: ConfigChoice[] } | null;
}

/// One config option as ACP puts it on the wire: the kind's fields are
/// flattened into the option (`type`, `currentValue`, `options`), and the
/// options of a select are either plain choices or groups of them.
interface WireConfigOption {
  id?: string;
  name?: string;
  category?: string;
  type?: string;
  currentValue?: string | boolean;
  options?: ({ value?: string; name?: string } | { group?: string; options?: { value?: string; name?: string }[] })[];
}

interface WireConfiguration {
  config_options?: WireConfigOption[] | null;
  legacy_modes?: { currentModeId?: string; availableModes?: { id?: string; name?: string }[] } | null;
}

/// The wire's shape of the session's configuration, read for the page.
export function readSessionOptions(wire: WireConfiguration): SessionOptions {
  const options = (wire.config_options ?? []).flatMap((option): ConfigOption[] => {
    if (!option.id) return [];
    const kind = option.type === "boolean" ? "boolean" : "select";
    const choices: ConfigChoice[] = [];
    for (const entry of option.options ?? []) {
      if ("options" in entry && Array.isArray(entry.options)) {
        for (const choice of entry.options) if (choice.value) choices.push({ value: choice.value, name: choice.name ?? choice.value });
      } else if ("value" in entry && entry.value) {
        choices.push({ value: entry.value, name: entry.name ?? entry.value });
      }
    }
    return [
      {
        id: option.id,
        name: option.name ?? option.id,
        category: option.category ?? null,
        kind,
        currentValue: option.currentValue ?? (kind === "boolean" ? false : ""),
        choices,
      },
    ];
  });
  const legacy = wire.legacy_modes;
  const modes =
    legacy && Array.isArray(legacy.availableModes)
      ? {
          current: legacy.currentModeId ?? "",
          available: legacy.availableModes.filter((mode) => mode.id).map((mode) => ({ value: mode.id ?? "", name: mode.name ?? mode.id ?? "" })),
        }
      : null;
  return { options, modes };
}

export interface SessionState {
  profiles: Profile[];
  /// What the agent lets this session configure; `null` before it is read.
  sessionOptions: SessionOptions | null;
  /// Every place a model is served from that a profile here may name.
  modelProviders: ModelProviderView[];
  /// What the open session is given with its next turns, as the host holds
  /// it; `null` when nothing is or no session is open. Connection-local: it
  /// dies with the connection.
  context: ModelContext | null;
  /// What an App last said a person is looking at, kept by the page so a
  /// session opened afterwards is given it too; `null` once let go of.
  offered: ModelContext | null;
  contextError: string;
  /// The sign-in method the person chose for the next start, by the agent's
  /// id for it; "" for the agent's own first choice.
  authMethodId: string;
  profileId: string;
  onboarding: Onboarding | null;
  onboardingStatus: string;
  /// Per profile id, what its agent advertised; `null` while unknown.
  handshakes: Record<string, Handshake | { error: string }>;
  /// Per profile id, the secrets it holds (names and kinds; never values).
  secrets: Record<string, ProfileSecrets>;
  /// The sessions of the selected profile, newest first.
  sessions: SessionSummary[];
  /// What the selected profile's agent has been handed and has handed back.
  files: HandedFile[];
  /// Why the last read of them failed, if it did. An empty list and a list
  /// that could not be read are different things to a person waiting for a
  /// file, and the panel must not say the first when it means the second.
  filesStatus: string;
  /// The standing instructions of the selected profile.
  schedules: Schedule[];
  /// Every MCP server this host can attach to an agent.
  mcpServers: McpServerView[];
  /// Whether the host has said what it declares yet: until it has, a server
  /// the agent attaches cannot be called missing.
  mcpServersKnown: boolean;
  permissionProfiles: PermissionProfileOption[];
  environments: EnvironmentProfileOption[];
  /// A sentence about the last thing that happened while configuring, or "".
  environmentStatus: string;
  /// Sequences the page read from the session's history rather than received
  /// on the live lane. The two are different facts: the lane must never
  /// re-deliver what this surface acknowledged, and the history is read on
  /// purpose so a resumed session opens on what was said in it.
  historySequences: number[];
  /// Who this person says they are, for the turns they write. Empty until
  /// they say; the record then keeps the surface alone, which is honest -
  /// nobody said. Remembered per browser, never sent anywhere else.
  writingAs: string;
  connectionId: string | null;
  routeId: string | null;
  routeInput: string;
  phase: Phase | null;
  /// A sentence about the last thing that went wrong, or "".
  problem: string;
  conversation: Conversation;
  events: SurfaceEvent[];
  permission: PermissionRequest | null;
  structured: StructuredInteraction | null;
  structuredError: string;
  apps: AppAttachment[];
  appsStatus: string;
  /// The App last opened on this connection; kept after it closes so the
  /// diagnostics still name it.
  opened: OpenedApp | null;
  /// Whether `opened` is mounted right now.
  appOpen: boolean;
  busy: boolean;
}

export type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export class SessionStore {
  private state: SessionState;
  private readonly listeners = new Set<() => void>();
  private generation = 0;
  private readonly fetchImpl: Fetch;
  private readonly storage: Storage | null;
  /// The mounted App's teardown, owned outside React's render.
  private mounted: { teardown(): Promise<void>; deliver?(terminal: unknown): Promise<void> } | null = null;
  private observationCursor = 0;

  constructor(fetchImpl: Fetch = (input, init) => fetch(input, init), storage: Storage | null = null) {
    this.fetchImpl = fetchImpl;
    this.storage = storage;
    this.state = {
      profiles: [],
      profileId: "",
      onboarding: null,
      onboardingStatus: "",
      handshakes: {},
      secrets: {},
      sessions: [],
      files: [],
      filesStatus: "",
      schedules: [],
      mcpServers: [],
      mcpServersKnown: false,
      permissionProfiles: [],
      environments: [],
      environmentStatus: "",
      modelProviders: [],
      authMethodId: "",
      context: null,
      offered: null,
      contextError: "",
      historySequences: [],
      writingAs: "",
      connectionId: null,
      routeId: null,
      routeInput: "",
      phase: null,
      problem: "",
      conversation: emptyConversation(),
      events: [],
      sessionOptions: null,
      permission: null,
      structured: null,
      structuredError: "",
      apps: [],
      appsStatus: "no connection",
      opened: null,
      appOpen: false,
      busy: false,
    };
  }

  // ---- store plumbing -----------------------------------------------------

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): SessionState => this.state;

  private set(patch: Partial<SessionState>): void {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }

  private remember(key: string, value: string): void {
    try {
      this.storage?.setItem(key, value);
    } catch {
      // Storage is a convenience.
    }
  }

  private recall(key: string): string | null {
    try {
      return this.storage?.getItem(key) ?? null;
    } catch {
      return null;
    }
  }

  /// Say who is writing from this browser. Remembered here, sent with each
  /// turn; cleared by emptying it.
  setWritingAs(name: string): void {
    this.set({ writingAs: name });
    this.remember(WRITING_AS_KEY, name);
  }

  /// What the agent lets this session configure, read from the host as it
  /// stands: after the session opens, and again whenever the agent says its
  /// options moved. A read that fails leaves the last one; the page never
  /// invents an option.
  async loadSessionOptions(connectionId: string): Promise<void> {
    try {
      const wire = await this.api<Parameters<typeof readSessionOptions>[0]>("GET", `/api/connections/${encodeURIComponent(connectionId)}/options`);
      if (this.state.connectionId !== connectionId) return;
      this.set({ sessionOptions: readSessionOptions(wire) });
    } catch {
      // The options are a convenience over the conversation; a read that
      // failed is retried at the next change.
    }
  }

  /// Choose one of the agent's config options - its model, its mode. The
  /// host answers the whole list as it stands afterwards, or the agent's own
  /// refusal, which becomes the problem line.
  async setSessionOption(configId: string, value: string | boolean): Promise<void> {
    const connectionId = this.state.connectionId;
    if (!connectionId) return;
    try {
      const answer = await this.api<{ config_options: Parameters<typeof readSessionOptions>[0]["config_options"] }>(
        "POST",
        `/api/connections/${encodeURIComponent(connectionId)}/options`,
        { config_id: configId, value },
      );
      this.set({
        sessionOptions: readSessionOptions({ config_options: answer.config_options, legacy_modes: null }),
        problem: "",
      });
    } catch (error) {
      this.set({ problem: `The agent did not take that: ${(error as Error).message}` });
    }
  }

  /// Choose one of an older agent's modes.
  async setSessionMode(modeId: string): Promise<void> {
    const connectionId = this.state.connectionId;
    if (!connectionId) return;
    try {
      const answer = await this.api<{ modes: NonNullable<Parameters<typeof readSessionOptions>[0]["legacy_modes"]> }>(
        "POST",
        `/api/connections/${encodeURIComponent(connectionId)}/mode`,
        { mode_id: modeId },
      );
      this.set({
        sessionOptions: {
          options: this.state.sessionOptions?.options ?? [],
          modes: readSessionOptions({ legacy_modes: answer.modes }).modes,
        },
        conversation: { ...this.state.conversation, mode: answer.modes.currentModeId ?? modeId },
        problem: "",
      });
    } catch (error) {
      this.set({ problem: `The agent did not take that: ${(error as Error).message}` });
    }
  }

  async api<T>(method: string, path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
    const init: RequestInit = { method, headers: { "content-type": "application/json" } };
    if (signal) init.signal = signal;
    if (body !== undefined) init.body = JSON.stringify(body);
    const response = await this.fetchImpl(path, init);
    const text = await response.text();
    const value = text ? (JSON.parse(text) as { error?: string }) : null;
    if (!response.ok) throw new Error(value?.error ?? String(response.status));
    return value as T;
  }

  // ---- profiles and onboarding -------------------------------------------

  async loadProfiles(preferred?: string): Promise<void> {
    const profiles = await this.api<Profile[]>("GET", "/api/profiles");
    const remembered = this.recall("swem.workbench.profile");
    const wanted = preferred ?? remembered ?? "";
    const profileId = profiles.some((profile) => profile.profile_id === wanted)
      ? wanted
      : (profiles[0]?.profile_id ?? "");
    this.set({
      profiles,
      profileId,
      routeInput: this.recall(`swem.workbench.route.${profileId}`) ?? "",
      writingAs: this.recall(WRITING_AS_KEY) ?? "",
    });
    if (profileId) this.remember("swem.workbench.profile", profileId);
    // The agents present on this computer are worth knowing even once a
    // profile exists: a person adds a second profile of one of them from the
    // rail, without going back through first run.
    await this.loadOnboarding();
    if (profileId) void this.loadHandshake(profileId);
  }

  selectProfile(profileId: string): void {
    this.set({ profileId, routeInput: this.recall(`swem.workbench.route.${profileId}`) ?? "" });
    this.remember("swem.workbench.profile", profileId);
    void this.loadHandshake(profileId);
  }

  // ---- the agent's environment -------------------------------------------

  /// Every MCP server this host can attach: the projects it serves and the
  /// servers a person declared here.
  async loadMcpServers(): Promise<void> {
    try {
      const answer = await this.api<{ servers: McpServerView[] }>("GET", "/api/mcp-servers");
      this.set({ mcpServers: answer.servers, mcpServersKnown: true, environmentStatus: "" });
    } catch (error) {
      this.set({ mcpServers: [], environmentStatus: (error as Error).message });
    }
  }

  /// The ways this host can answer an agent's permission requests.
  async loadPermissionProfiles(): Promise<void> {
    try {
      const answer = await this.api<{ profiles: PermissionProfileOption[] }>(
        "GET", "/api/permission-profiles");
      this.set({ permissionProfiles: answer.profiles });
    } catch {
      // The rest of the panel is still usable; the choice simply does not
      // appear rather than the page breaking around it.
      this.set({ permissionProfiles: [] });
    }
  }

  /// Where this host can run an agent.
  async loadEnvironments(): Promise<void> {
    try {
      const answer = await this.api<{ environments: EnvironmentProfileOption[] }>(
        "GET", "/api/environments");
      this.set({ environments: answer.environments });
    } catch {
      this.set({ environments: [] });
    }
  }

  /// Declare a server, or correct one already declared.
  async declareMcpServer(body: DeclareMcpServer): Promise<boolean> {
    this.set({ environmentStatus: "Saving…" });
    try {
      await this.api<McpServerView>("POST", "/api/mcp-servers", body);
      await this.loadMcpServers();
      return true;
    } catch (error) {
      this.set({ environmentStatus: (error as Error).message });
      return false;
    }
  }

  /// Forget a declared server. The host refuses while an agent attaches it.
  async forgetMcpServer(name: string): Promise<void> {
    this.set({ environmentStatus: "" });
    try {
      await this.api("DELETE", `/api/mcp-servers/${encodeURIComponent(name)}`);
      await this.loadMcpServers();
    } catch (error) {
      this.set({ environmentStatus: (error as Error).message });
    }
  }

  /// The profile a person is configuring, as the host last sent it.
  profile(profileId?: string): Profile | null {
    const wanted = profileId ?? this.state.profileId;
    return this.state.profiles.find((profile) => profile.profile_id === wanted) ?? null;
  }

  /// Change what the selected profile is - where it works, what it reaches -
  /// keeping its identity and the sessions already bound to it.
  async amendProfile(changes: {
    workspace?: string;
    agent_home?: string;
    attachments?: string[];
    permissions?: string;
    environment?: string;
    setup?: AgentSetup;
  }): Promise<boolean> {
    const profile = this.profile();
    if (!profile) {
      this.set({ environmentStatus: "Choose an agent first." });
      return false;
    }
    this.set({ environmentStatus: "Saving…", busy: true });
    try {
      await this.api<Profile>("PATCH", `/api/profiles/${encodeURIComponent(profile.profile_id)}`, {
        revision: profile.revision ?? 0,
        ...changes,
      });
      await this.loadProfiles(profile.profile_id);
      this.set({ environmentStatus: "Saved." });
      return true;
    } catch (error) {
      this.set({ environmentStatus: (error as Error).message });
      return false;
    } finally {
      this.set({ busy: false });
    }
  }

  /// Every place a model is served from, shipped and declared.
  async loadModelProviders(): Promise<void> {
    try {
      const answer = await this.api<{ providers: ModelProviderView[] }>("GET", "/api/model-providers");
      this.set({ modelProviders: answer.providers });
    } catch (error) {
      this.set({ modelProviders: [], environmentStatus: (error as Error).message });
    }
  }

  /// Declare a provider, or correct one; the list is read again afterwards.
  async declareModelProvider(body: DeclareModelProvider): Promise<boolean> {
    this.set({ environmentStatus: "Saving…" });
    try {
      await this.api<ModelProviderView>("POST", "/api/model-providers", body);
      await this.loadModelProviders();
      this.set({ environmentStatus: "Saved." });
      return true;
    } catch (error) {
      this.set({ environmentStatus: (error as Error).message });
      return false;
    }
  }

  async forgetModelProvider(id: string): Promise<void> {
    try {
      await this.api("DELETE", `/api/model-providers/${encodeURIComponent(id)}`);
      await this.loadModelProviders();
      this.set({ environmentStatus: "" });
    } catch (error) {
      this.set({ environmentStatus: (error as Error).message });
    }
  }

  /// The setup the selected profile carries, as one value.
  profileSetup(): AgentSetup {
    const profile = this.profile();
    return {
      model_provider: profile?.model_provider ?? null,
      model: profile?.model ?? null,
      role: profile?.role ?? "",
      agent_skills: profile?.agent_skills ?? [],
    };
  }

  /// Change part of the selected profile's setup; the whole setup goes back
  /// to the host, because that is what the form shows.
  async setProfileSetup(changes: Partial<AgentSetup>): Promise<boolean> {
    return this.amendProfile({ setup: { ...this.profileSetup(), ...changes } });
  }

  /// What the open session is given, as the host holds it. Read when a
  /// session opens; what an App said before that is handed over then.
  async loadContext(): Promise<void> {
    const connectionId = this.state.connectionId;
    if (!connectionId) {
      this.set({ context: null });
      return;
    }
    if (this.state.offered) {
      await this.bindContext(this.state.offered);
      return;
    }
    try {
      const answer = await this.api<{ context: ModelContext | null }>("GET", `/api/connections/${encodeURIComponent(connectionId)}/context`);
      if (this.state.connectionId === connectionId) this.set({ context: answer.context });
    } catch {
      if (this.state.connectionId === connectionId) this.set({ context: null });
    }
  }

  /// An App of `server` said what a person is looking at
  /// (`ui/update-model-context`). Each update replaces the one before; one
  /// that says nothing lets go. The open session is given it with its next
  /// turn; without a session the page keeps it for the one opened next.
  async offerContext(server: string, update: { content?: unknown[] }): Promise<void> {
    const content = contextBlocks(update.content);
    if (content.length === 0) {
      if (this.state.offered?.server_name === server || this.state.context?.server_name === server) {
        await this.clearContext();
      }
      return;
    }
    const offered: ModelContext = { server_name: server, content };
    this.set({ offered });
    await this.bindContext(offered);
  }

  private async bindContext(body: ModelContext): Promise<void> {
    const connectionId = this.state.connectionId;
    if (!connectionId) return;
    this.set({ contextError: "" });
    try {
      const context = await this.api<ModelContext>("POST", `/api/connections/${encodeURIComponent(connectionId)}/context`, body);
      this.set({ context });
    } catch (error) {
      this.set({ context: null, contextError: (error as Error).message });
    }
  }

  async clearContext(): Promise<void> {
    this.set({ offered: null });
    const connectionId = this.state.connectionId;
    if (!connectionId) {
      this.set({ context: null, contextError: "" });
      return;
    }
    try {
      await this.api<{ cleared: boolean }>("DELETE", `/api/connections/${encodeURIComponent(connectionId)}/context`);
      this.set({ context: null, contextError: "" });
    } catch (error) {
      this.set({ contextError: (error as Error).message });
    }
  }

  /// Which of the agent's sign-in methods the next start uses.
  setAuthMethod(authMethodId: string): void {
    if (authMethodId !== this.state.authMethodId) this.set({ authMethodId });
  }

  setRouteInput(routeInput: string): void {
    this.set({ routeInput });
  }

  async loadOnboarding(): Promise<void> {
    try {
      const onboarding = await this.api<Onboarding>("GET", "/api/onboarding");
      this.set({
        onboarding,
        onboardingStatus: onboarding.enabled ? "" : "Create a profile with the host API to begin.",
      });
    } catch (error) {
      this.set({ onboardingStatus: (error as Error).message });
    }
  }

  /// Make a profile of an agent this computer already has. `profileId` is the
  /// name a person gives it; first run leaves it out and the profile is named
  /// after the agent. A second profile of the same agent gets its own
  /// workspace and its own native home, so two people are not one.
  async useAgent(agentId: string, profileId?: string, setup?: Partial<AgentSetup>): Promise<void> {
    this.set({ onboardingStatus: "Creating local profile…" });
    try {
      // A setup given with the name goes with the create; one click on an
      // agent gives none and the agent's own defaults stand.
      const filled = setup && (setup.model_provider || setup.model || (setup.role ?? "").trim())
        ? { setup: { model_provider: setup.model_provider ?? null, model: setup.model ?? null, role: setup.role ?? "", agent_skills: setup.agent_skills ?? [] } }
        : {};
      const profile = await this.api<Profile>("POST", "/api/profiles/local", {
        agent_id: agentId,
        ...(profileId ? { profile_id: profileId } : {}),
        ...filled,
      });
      this.set({ onboardingStatus: "" });
      await this.loadProfiles(profile.profile_id);
    } catch (error) {
      this.set({ onboardingStatus: (error as Error).message });
    }
  }

  /// Install from the exact plan the person is shown. `confirm` is the
  /// consent step, injected so a test can answer it.
  async installAgent(agentId: string, confirm: (question: string) => boolean): Promise<void> {
    this.set({ onboardingStatus: "Reading the install plan…" });
    let plan: {
      plan_id: string;
      registry_id: string;
      version: string;
      registry_index?: string;
      distribution?: Record<string, unknown>;
    };
    try {
      plan = await this.api("GET", `/api/agents/${encodeURIComponent(agentId)}/install-plan`);
    } catch (error) {
      this.set({ onboardingStatus: (error as Error).message });
      return;
    }
    // What the person is consenting to is the plan, so the question names the
    // thing that will be fetched and where the plan came from. The index is
    // not always the public one any more - a mirror, an internal copy or a
    // file is a person's own choice - and consenting to an install without
    // being told which index described it is consenting to a word.
    const distribution = (plan.distribution ?? {}) as {kind?: string; package?: string; archive?: string};
    const what = distribution.package ?? distribution.archive ?? distribution.kind ?? "distribution";
    if (
      !confirm(
        `Install ${plan.registry_id} ${plan.version}?\n\n` +
          `It fetches ${what}, described by ${plan.registry_index ?? "the agent registry"}, ` +
          "into this product's agent home. Nothing else on this machine changes.",
      )
    ) {
      this.set({ onboardingStatus: "" });
      return;
    }
    this.set({ onboardingStatus: `Installing ${plan.registry_id}…` });
    try {
      await this.api("POST", "/api/agents/install", { agent_id: agentId, plan_id: plan.plan_id });
      this.set({ onboardingStatus: "Installed. Creating local profile…" });
      await this.loadOnboarding();
      await this.useAgent(agentId);
    } catch (error) {
      this.set({ onboardingStatus: (error as Error).message });
    }
  }

  // ---- authentication -----------------------------------------------------

  /// Ask the agent what it will need, before offering to start a session.
  /// What the agent has been handed and what it has handed back. Read after
  /// every turn, because the agent writing a file is how it hands one back:
  /// there is no event for it and there should not be, since the whole point
  /// of a directory is that an agent needs to know nothing about us to use it.
  async loadFiles(profileId?: string): Promise<void> {
    const wanted = profileId ?? this.state.profileId;
    if (!wanted) {
      this.set({ files: [], filesStatus: "" });
      return;
    }
    try {
      const files = await this.api<HandedFile[]>(
        "GET",
        `/api/profiles/${encodeURIComponent(wanted)}/files`,
      );
      if (this.state.profileId === wanted) this.set({ files, filesStatus: "" });
    } catch (error) {
      // Not "there are none": this read did not happen. Saying nothing is
      // here would be the page answering a question it never asked the host.
      if (this.state.profileId === wanted) {
        this.set({ filesStatus: `These could not be read: ${(error as Error).message}` });
      }
    }
  }

  /// The standing instructions this profile's clock runs.
  async loadSchedules(profileId?: string): Promise<void> {
    const wanted = profileId ?? this.state.profileId;
    if (!wanted) {
      this.set({ schedules: [] });
      return;
    }
    try {
      const schedules = await this.api<Schedule[]>(
        "GET",
        `/api/profiles/${encodeURIComponent(wanted)}/schedules`,
      );
      if (this.state.profileId === wanted) this.set({ schedules });
    } catch {
      // A host that keeps no schedules simply shows none.
      if (this.state.profileId === wanted) this.set({ schedules: [] });
    }
  }

  /// Set one. It runs once straight away, so a person sees what it does
  /// rather than waiting an interval to find out.
  async setSchedule(scheduleId: string, say: string, everyMinutes: number): Promise<boolean> {
    const profileId = this.state.profileId;
    if (!profileId) return false;
    this.set({ environmentStatus: "Saving…" });
    try {
      await this.api("POST", `/api/profiles/${encodeURIComponent(profileId)}/schedules`, {
        schedule_id: scheduleId,
        say,
        every_minutes: everyMinutes,
      });
      await this.loadSchedules(profileId);
      this.set({ environmentStatus: "Saved." });
      return true;
    } catch (error) {
      this.set({ environmentStatus: (error as Error).message });
      return false;
    }
  }

  async forgetSchedule(scheduleId: string): Promise<void> {
    try {
      await this.api("DELETE", `/api/schedules/${encodeURIComponent(scheduleId)}`);
      await this.loadSchedules();
    } catch (error) {
      this.set({ environmentStatus: (error as Error).message });
    }
  }

  async loadSessions(profileId: string): Promise<void> {
    if (!profileId) {
      this.set({ sessions: [] });
      return;
    }
    try {
      const sessions = await this.api<SessionSummary[]>(
        "GET",
        `/api/profiles/${encodeURIComponent(profileId)}/sessions`,
      );
      if (this.state.profileId === profileId) this.set({ sessions });
    } catch {
      // A profile with no ledger yet simply has no sessions.
      if (this.state.profileId === profileId) this.set({ sessions: [] });
    }
  }

  /// Open one of this agent's earlier chats: what was said in it, read from
  /// the record. No agent is started for reading. One that is already
  /// running on this chat is joined; otherwise the agent starts when the
  /// person writes.
  async resumeSession(routeId: string): Promise<void> {
    await this.readChat(routeId);
    const generation = this.generation;
    try {
      const live = await this.api<{ connection_id: string; route_id: string } | null>(
        "GET",
        `/api/routes/${encodeURIComponent(routeId)}/connection?profile_id=${encodeURIComponent(this.state.profileId)}`,
      );
      if (!live || generation !== this.generation) return;
      this.set({ connectionId: live.connection_id });
      void this.elicitationLoop(generation, live.connection_id);
      this.showConnection(live, generation);
    } catch {
      // Nothing is running on it: the chat is read, and that is enough.
    }
  }

  private async readChat(routeId: string, problem = ""): Promise<void> {
    this.resetSurface();
    this.set({ routeId, routeInput: routeId, problem });
    this.remember(`swem.workbench.route.${this.state.profileId}`, routeId);
    await this.loadHistory(routeId, this.generation);
  }

  async loadHandshake(profileId: string): Promise<void> {
    if (!profileId) return;
    void this.loadSessions(profileId);
    void this.loadFiles(profileId);
    void this.loadSchedules(profileId);
    try {
      const [handshake, secrets] = await Promise.all([
        this.api<Handshake>("GET", `/api/profiles/${encodeURIComponent(profileId)}/handshake`),
        this.api<ProfileSecrets>("GET", `/api/profiles/${encodeURIComponent(profileId)}/secrets`),
      ]);
      this.set({
        handshakes: { ...this.state.handshakes, [profileId]: handshake },
        secrets: { ...this.state.secrets, [profileId]: secrets },
      });
    } catch (error) {
      this.set({ handshakes: { ...this.state.handshakes, [profileId]: { error: (error as Error).message } } });
    }
  }

  /// Put a typed secret in the profile's vault. The value goes to the host
  /// once and is never held in this store afterwards.
  async saveSecret(profileId: string, secret: NewSecret): Promise<void> {
    const secrets = await this.api<ProfileSecrets>("PUT", `/api/profiles/${encodeURIComponent(profileId)}/secrets`, secret);
    this.set({ secrets: { ...this.state.secrets, [profileId]: secrets } });
  }

  /// Forget one secret by the variable name it is injected under.
  async removeSecret(profileId: string, name: string): Promise<void> {
    const secrets = await this.api<ProfileSecrets>(
      "DELETE",
      `/api/profiles/${encodeURIComponent(profileId)}/secrets/${encodeURIComponent(name)}`,
    );
    this.set({ secrets: { ...this.state.secrets, [profileId]: secrets } });
  }

  /// The selected profile's vault, or an empty one while unknown.
  profileSecrets(): ProfileSecrets {
    return this.state.secrets[this.state.profileId] ?? { types: [], secrets: [] };
  }

  /// The methods the selected profile's agent advertised, if known.
  authMethods(): AuthMethod[] {
    const handshake = this.state.handshakes[this.state.profileId];
    return handshake && !("error" in handshake) ? handshake.auth_methods : [];
  }

  // ---- connection lifecycle ----------------------------------------------

  private resetSurface(): void {
    this.generation += 1;
    void this.closeApp(true);
    this.observationCursor = 0;
    this.set({
      connectionId: null,
      routeId: null,
      phase: null,
      problem: "",
      conversation: emptyConversation(),
      events: [],
      sessionOptions: null,
      context: null,
      contextError: "",
      historySequences: [],
      permission: null,
      structured: null,
      structuredError: "",
      apps: [],
      appsStatus: "connecting…",
      opened: null,
      appOpen: false,
    });
  }

  async open(mode: "new" | "load" | "resume", authMethodId?: string): Promise<void> {
    this.set({ busy: true });
    try {
      await this.openConnection(mode, authMethodId);
    } catch (error) {
      this.generation += 1;
      this.set({
        connectionId: null,
        phase: { phase: "finished" },
        problem: `The session could not start: ${(error as Error).message}`,
      });
    } finally {
      this.set({ busy: false });
    }
  }

  private async openConnection(mode: "new" | "load" | "resume", authMethodId?: string): Promise<void> {
    this.resetSurface();
    const { profileId } = this.state;
    const enteredRoute = this.state.routeInput.trim();
    if (mode !== "new" && enteredRoute) {
      const live = await this.api<{ connection_id: string; route_id: string } | null>(
        "GET",
        `/api/routes/${encodeURIComponent(enteredRoute)}/connection?profile_id=${encodeURIComponent(profileId)}`,
      );
      if (live) {
        this.set({ connectionId: live.connection_id });
        const generation = this.generation;
        void this.elicitationLoop(generation, live.connection_id);
        this.showConnection(live, generation);
        return;
      }
    }
    const proposed = `wb-${Date.now()}-${Math.random().toString(36).slice(2, 12)}`;
    const body: Record<string, unknown> = { profile_id: profileId, mode, connection_id: proposed };
    // A new session never names a route. The host binds a route to the exact
    // native session it was opened with, so handing it the route a previous
    // session used could only end as drift - which is what a person met the
    // second time they pressed Start.
    if (enteredRoute && mode !== "new") body["route_id"] = enteredRoute;
    if (authMethodId) body["auth_method_id"] = authMethodId;
    // Request-scoped elicitation may arrive during the open itself, before
    // the POST returns: the exact handle is registered in both peers and
    // long-polled while the open is still in flight.
    this.set({ connectionId: proposed });
    this.generation += 1;
    const generation = this.generation;
    void this.elicitationLoop(generation, proposed);
    const opened = await this.api<{ connection_id: string; route_id: string }>("POST", "/api/connections", body);
    if (opened.connection_id !== proposed) throw new Error("host changed the requested ephemeral connection handle");
    this.showConnection(opened, generation);
  }

  /// What was said in this session before now. The live lane only ever
  /// delivers what is past this surface's cursor, so without this a resumed
  /// session opened on an empty page.
  private async loadHistory(routeId: string, generation: number): Promise<void> {
    try {
      const read = await this.api<{ events: SurfaceEvent[] }>(
        "GET",
        `/api/routes/${encodeURIComponent(routeId)}/history?limit=1000`,
      );
      if (generation !== this.generation) return;
      let conversation = emptyConversation();
      for (const event of read.events) conversation = reduceEvent(conversation, event);
      // Live events that already arrived stay ahead of the history.
      const live = this.state.events.filter((event) => !read.events.some((old) => old.sequence === event.sequence));
      const delivered = new Set(this.state.events.map((event) => event.sequence));
      for (const event of live) conversation = reduceEvent(conversation, event);
      this.set({
        conversation,
        events: [...read.events, ...live],
        historySequences: read.events
          .map((event) => event.sequence)
          .filter((sequence) => !delivered.has(sequence)),
      });
    } catch {
      // A session with no lane yet simply shows nothing.
    }
  }

  private showConnection(opened: { route_id: string }, generation: number): void {
    const { profileId, connectionId } = this.state;
    this.set({ routeId: opened.route_id, routeInput: opened.route_id });
    void this.loadHistory(opened.route_id, generation);
    void this.loadSessions(profileId);
    this.remember("swem.workbench.profile", profileId);
    this.remember(`swem.workbench.route.${profileId}`, opened.route_id);
    if (!connectionId) return;
    void this.eventLoop(generation, opened.route_id);
    void this.permissionLoop(generation, connectionId);
    void this.phaseLoop(generation, connectionId);
    void this.observationLoop(generation, connectionId);
    void this.refreshApps(connectionId);
    void this.loadSessionOptions(connectionId);
    void this.loadContext();
  }

  private live(generation: number, connection: string): boolean {
    return generation === this.generation && this.state.connectionId === connection;
  }

  private async eventLoop(generation: number, route: string): Promise<void> {
    while (generation === this.generation && this.state.routeId === route) {
      let batch: { events: SurfaceEvent[] };
      try {
        batch = await this.api("GET", `/api/routes/${encodeURIComponent(route)}/events?surface_id=${SURFACE_ID}&wait_ms=10000`);
      } catch {
        await sleep(500);
        continue;
      }
      if (generation !== this.generation) return;
      if (batch.events.length === 0) {
        // A host that answers an empty poll at once must not starve the
        // rest of the page: give the event loop a turn before asking again.
        await sleep(0);
        continue;
      }
      let conversation = this.state.conversation;
      for (const event of batch.events) conversation = reduceEvent(conversation, event);
      const delivered = new Set(batch.events.map((event) => event.sequence));
      const patch: Partial<SessionState> = {
        conversation,
        events: [...this.state.events, ...batch.events],
        // An event the lane delivered is a delivery, even if the history read
        // happened to have raced ahead of it and shown it first.
        historySequences: this.state.historySequences.filter((sequence) => !delivered.has(sequence)),
      };
      const turnJustEnded = conversation.turnEnded && !this.state.conversation.turnEnded;
      if (turnJustEnded) patch.permission = null;
      // The agent said its options moved (or the session just opened): read
      // them from the host, which holds them as they stand.
      const optionsMoved = conversation.optionsVersion !== this.state.conversation.optionsVersion;
      this.set(patch);
      if (optionsMoved && this.state.connectionId) void this.loadSessionOptions(this.state.connectionId);
      // The rail's card for this session is a read of the record, taken when
      // the session opened. A session that has since been talked in still
      // read "Nothing said yet" there, about a conversation on the screen
      // beside it. The record is what fills that card, so it is read again at
      // the end of the turn that changed it.
      if (turnJustEnded) void this.loadSessions(this.state.profileId);
      const cursor = batch.events[batch.events.length - 1]?.sequence ?? 0;
      // Acknowledge only after rendering: the cursor is this surface's place.
      await this.api("POST", `/api/routes/${encodeURIComponent(route)}/ack`, { surface_id: SURFACE_ID, cursor });
    }
  }

  private async permissionLoop(generation: number, connection: string): Promise<void> {
    let after = 0;
    while (this.live(generation, connection)) {
      let request: PermissionRequest | null;
      try {
        request = await this.api("GET", `/api/connections/${connection}/permissions/next?wait_ms=10000&after=${after}`);
      } catch {
        await sleep(500);
        continue;
      }
      if (!this.live(generation, connection)) return;
      if (!request) continue;
      this.set({ permission: request });
      after = request.sequence;
    }
  }

  async selectPermission(optionId: string): Promise<void> {
    const { permission, connectionId } = this.state;
    if (!permission || !connectionId) return;
    await this.api("POST", `/api/connections/${connectionId}/permissions/select`, {
      sequence: permission.sequence,
      option_id: optionId,
    });
    this.set({ permission: null });
  }

  private async phaseLoop(generation: number, connection: string): Promise<void> {
    while (this.live(generation, connection)) {
      try {
        const status = await this.api<{ phase: Phase }>("GET", `/api/connections/${connection}`);
        const was = this.state.phase?.phase ?? null;
        this.set({ phase: status.phase });
        // The rail's card is a read of the record, and it is re-read when a
        // turn ends. A turn that dies mid-prompt does not end - there is no
        // `acp/prompt_response` for it - so the card kept the words it was
        // opened with while the panel beside it said the session ended with
        // an error: "Nothing said yet - 5 recorded", an inch from the
        // conversation it was said in. That is the moment a person most
        // needs the two to agree, so the record is re-read when the session
        // stops running, however it stopped.
        if (status.phase.phase === "finished" && was !== "finished") {
          void this.loadSessions(this.state.profileId);
        }
      } catch {
        return;
      }
      await sleep(750);
    }
  }

  private async elicitationLoop(generation: number, connection: string): Promise<void> {
    let after = 0;
    while (this.live(generation, connection)) {
      let offered: { sequence: number; request: Record<string, unknown> } | null;
      try {
        offered = await this.api("GET", `/api/connections/${connection}/elicitations/next?wait_ms=10000&after=${after}`);
      } catch {
        if (!this.live(generation, connection)) return;
        await sleep(100);
        continue;
      }
      if (!this.live(generation, connection)) return;
      if (!offered) continue;
      // One modal at a time.
      while (this.state.structured && generation === this.generation) await sleep(100);
      if (generation !== this.generation) return;
      const request = offered.request ?? {};
      const profile = this.state.profiles.find((candidate) => candidate.profile_id === this.state.profileId);
      const who = profile ? `${profile.profile_id} (${profile.agent_id})` : "unknown agent profile";
      const scope =
        typeof request["sessionId"] === "string"
          ? `session ${request["sessionId"]}`
          : request["requestId"] !== undefined
            ? `request ${String(request["requestId"])}`
            : "unknown scope";
      try {
        if (request["mode"] === "form") {
          this.set({
            structured: {
              phase: "acp_form",
              sequence: offered.sequence,
              context: `ACP agent ${who} · ${scope}`,
              message: typeof request["message"] === "string" ? request["message"] : "The agent requests structured input.",
              schema: request["requestedSchema"] ?? {},
            },
            structuredError: "",
          });
        } else if (request["mode"] === "url") {
          const url = String(request["url"]);
          const target = new URL(url);
          this.set({
            structured: {
              phase: "acp_url",
              sequence: offered.sequence,
              url,
              context: `ACP agent ${who} · ${scope} · target host ${target.host} · full URL ${url}`,
              message: typeof request["message"] === "string" ? request["message"] : "The agent requests consent to open this URL.",
              schema: { type: "object", properties: {} },
            },
            structuredError: "",
          });
        } else {
          await this.api("POST", `/api/connections/${connection}/elicitations/answer`, { sequence: offered.sequence, action: "cancel" });
        }
      } catch (error) {
        await this.api("POST", `/api/connections/${connection}/elicitations/answer`, { sequence: offered.sequence, action: "cancel" }).catch(() => {});
        this.set({ appsStatus: `ACP elicitation refused: ${(error as Error).message}`, structured: null });
      }
      after = offered.sequence;
    }
  }

  // ---- structured interactions (ACP elicitation and App fallbacks) ---------

  openStructuredInteraction(serverName: string, tool: AppTool): void {
    this.set({
      structured: {
        phase: "launch",
        serverName,
        tool: { name: tool.name, input_schema: tool.input_schema, ...(tool.description ? { description: tool.description } : {}) },
        context: `server ${serverName} · tool ${tool.name} · independent_host_connection`,
        message: tool.description ?? "Start the server-owned structured fallback.",
        schema: tool.input_schema,
      },
      structuredError: "",
    });
  }

  /// `value` is the form's coerced answer for an accept; ignored otherwise.
  async answerStructured(action: "accept" | "decline" | "cancel", value: () => Record<string, unknown>): Promise<void> {
    const current = this.state.structured;
    const connection = this.state.connectionId;
    if (!current || !connection) return;
    this.set({ structuredError: "" });
    try {
      if (current.phase === "acp_form" || current.phase === "acp_url") {
        const body: Record<string, unknown> = { sequence: current.sequence, action };
        if (action === "accept" && current.phase === "acp_form") body["content"] = value();
        await this.api("POST", `/api/connections/${connection}/elicitations/answer`, body);
        if (action === "accept" && current.phase === "acp_url" && current.url) {
          window.open(current.url, "_blank", "noopener,noreferrer");
        }
        this.set({ structured: null });
        return;
      }
      if (current.phase === "launch") {
        if (action === "cancel") {
          this.set({ structured: null });
          return;
        }
        const pending = await this.api<{
          interaction_id: string;
          server_name: string;
          tool: string;
          connection_scope: string;
          message: string;
          requested_schema: unknown;
        }>("POST", `/api/connections/${connection}/apps/interactions`, {
          server_name: current.serverName,
          tool: current.tool?.name,
          arguments: value(),
        });
        this.set({
          structured: {
            ...current,
            phase: "elicitation",
            interactionId: pending.interaction_id,
            context: `server ${pending.server_name} · tool ${pending.tool} · ${pending.connection_scope}`,
            message: pending.message,
            schema: pending.requested_schema,
          },
        });
        return;
      }
      const body: Record<string, unknown> = { action };
      if (action === "accept") body["content"] = value();
      const result = await this.api<{ structuredContent?: { view_digest?: string } }>(
        "POST",
        `/api/connections/${connection}/apps/interactions/${encodeURIComponent(current.interactionId ?? "")}`,
        body,
      );
      const digest = result?.structuredContent?.view_digest;
      this.set({ appsStatus: `structured interaction ${action}${digest ? ` · ${digest}` : ""}`, structured: null });
    } catch (error) {
      this.set({ structuredError: (error as Error).message });
    }
  }

  // ---- prompting ----------------------------------------------------------

  private async uploadFile(file: File): Promise<{ descriptor_id: string }> {
    const response = await this.fetchImpl(`/api/content?name=${encodeURIComponent(file.name)}`, {
      method: "POST",
      headers: { "content-type": file.type || "application/octet-stream" },
      body: file,
    });
    const value = (await response.json()) as { descriptor_id: string; error?: string };
    if (!response.ok) throw new Error(value?.error ?? String(response.status));
    return value;
  }

  async sendPrompt(input: {
    text: string;
    files: File[];
    link?: { uri: string; name?: string; mimeType?: string };
    embed?: { uri: string; text: string };
  }): Promise<void> {
    let connection = this.state.connectionId;
    const reading = this.state.routeId;
    if (!connection && reading) {
      // A chat that was being read: the agent starts now, on the first
      // thing the person says, and goes on from where the chat stopped.
      this.setRouteInput(reading);
      await this.open("resume");
      connection = this.state.connectionId;
      if (!connection) {
        // It could not start. The chat is still there to read, with why.
        const why = this.state.problem;
        await this.readChat(reading, why);
        throw new Error(why);
      }
    }
    if (!connection) return;
    // A resource is its URI: a name, a MIME type or a body without one has
    // nothing to hang on, and the blocks below would leave it out. What the
    // person typed is theirs, so they are told rather than sent a turn that
    // quietly does not carry it.
    const orphan =
      (!input.link?.uri && (input.link?.name || input.link?.mimeType) && "resource link") ||
      (!input.embed?.uri && input.embed?.text && "embedded resource");
    if (orphan) {
      const refusal = `turn failed: the ${orphan} needs its URI`;
      this.set({ conversation: { ...this.state.conversation, turnOutcome: refusal } });
      throw new Error(refusal);
    }
    this.set({ conversation: { ...this.state.conversation, turnOutcome: "…" } });
    try {
      const content: unknown[] = [];
      if (input.text) content.push({ type: "text", text: input.text });
      if (input.link?.uri) {
        content.push({
          type: "resource_link",
          uri: input.link.uri,
          name: input.link.name || input.link.uri,
          mimeType: input.link.mimeType || undefined,
        });
      }
      if (input.embed?.uri) content.push({ type: "resource", resource: { uri: input.embed.uri, text: input.embed.text } });
      const uploaded = await Promise.all(input.files.map((file) => this.uploadFile(file)));
      const outcome = await this.api<{ stop_reason: string; control_outcome: string }>(
        "POST",
        `/api/connections/${connection}/prompt`,
        {
          content,
          content_refs: uploaded.map((descriptor) => descriptor.descriptor_id),
          // Who wrote this, and from where. ACP has no field for it, so the
          // host puts it in the content the agent reads and in the record.
          correspondent: { surface: SURFACE_ID, ...(this.state.writingAs ? { author: this.state.writingAs } : {}) },
        },
      );
      this.set({
        conversation: { ...this.state.conversation, turnOutcome: `turn: ${outcome.stop_reason} / ${outcome.control_outcome}` },
      });
      void this.loadFiles();
    } catch (error) {
      this.set({ conversation: { ...this.state.conversation, turnOutcome: `turn failed: ${(error as Error).message}` } });
      throw error;
    }
  }

  cancel(): void {
    const connection = this.state.connectionId;
    if (connection) void this.api("POST", `/api/connections/${connection}/cancel`, {}).catch(() => {});
  }

  async disconnect(): Promise<void> {
    await this.end("disconnect");
  }

  async close(): Promise<void> {
    await this.end("close");
  }

  private async end(how: "disconnect" | "close"): Promise<void> {
    const connection = this.state.connectionId;
    if (!connection) return;
    this.set({ problem: "" });
    try {
      const result = await this.api<{ termination: string }>("POST", `/api/connections/${connection}/${how}`, {});
      this.generation += 1;
      this.set({
        phase: { phase: "finished" },
        connectionId: null,
        conversation: {
          ...this.state.conversation,
          terminal: { status: "finished", summary: `Session ended (${result.termination}).`, detail: null, needsAuthentication: false },
        },
      });
    } catch (error) {
      this.set({ problem: `close refused: ${(error as Error).message}` });
    }
  }

  // ---- Apps ---------------------------------------------------------------

  async refreshApps(connection: string): Promise<void> {
    try {
      const apps = await this.api<AppAttachment[]>("GET", `/api/connections/${connection}/apps`);
      const declared = apps.reduce((count, attachment) => count + attachment.apps.length, 0);
      this.set({ apps, appsStatus: declared === 0 ? "no Apps declared" : `${declared} App(s) declared` });
    } catch (error) {
      this.set({ appsStatus: `apps discovery failed: ${(error as Error).message}` });
    }
  }

  setAppsStatus(appsStatus: string): void {
    this.set({ appsStatus });
  }

  /// `mount` is the bridge call, injected because it needs a DOM container.
  async openApp(
    serverName: string,
    uri: string,
    mount: (opened: OpenedApp, relay: (message: unknown) => Promise<unknown>, observation: unknown) => Promise<{ teardown(): Promise<void>; deliver?(terminal: unknown): Promise<void> } | null>,
  ): Promise<void> {
    const connection = this.state.connectionId;
    if (!connection) return;
    const opened = await this.api<OpenedApp>("POST", `/api/connections/${connection}/apps/open`, { server_name: serverName, uri });
    await this.mountOpened(connection, opened, null, mount);
  }

  private async mountOpened(
    connection: string,
    opened: OpenedApp,
    observation: { status?: string; observation_id?: string } | null,
    mount: (opened: OpenedApp, relay: (message: unknown) => Promise<unknown>, observation: unknown) => Promise<{ teardown(): Promise<void>; deliver?(terminal: unknown): Promise<void> } | null>,
  ): Promise<void> {
    if (this.mounted) await this.closeApp();
    this.set({ opened, appOpen: true });
    const relay = (message: unknown) => this.api<unknown>("POST", `/api/connections/${connection}/apps/${opened.app_id}/rpc`, message);
    const mounted = await mount(opened, relay, observation);
    if (!mounted) return;
    this.mounted = mounted;
    if (observation?.status === "pending" && observation.observation_id) {
      const terminal = await this.api<unknown>(
        "GET",
        `/api/connections/${connection}/apps/observations/${encodeURIComponent(observation.observation_id)}?wait_ms=25000`,
      );
      if (terminal && this.mounted === mounted) await mounted.deliver?.(terminal);
    }
  }

  /// The mount used by the observation loop, set by the surface that owns
  /// the container.
  mountObserved:
    | ((opened: OpenedApp, relay: (message: unknown) => Promise<unknown>, observation: unknown) => Promise<{ teardown(): Promise<void>; deliver?(terminal: unknown): Promise<void> } | null>)
    | null = null;

  private async observationLoop(generation: number, connection: string): Promise<void> {
    let after = this.observationCursor;
    while (this.live(generation, connection)) {
      let observed: { opened: OpenedApp; observation: { cursor: number; status?: string; observation_id?: string } } | null;
      try {
        observed = await this.api("GET", `/api/connections/${connection}/apps/observations/next?after=${after}&wait_ms=10000`);
      } catch {
        return;
      }
      if (!this.live(generation, connection)) return;
      if (!observed) continue;
      after = observed.observation.cursor;
      this.observationCursor = after;
      if (this.mountObserved) await this.mountOpened(connection, observed.opened, observed.observation, this.mountObserved);
    }
  }

  /// `quiet` tears the App down without reporting it, for a surface reset.
  async closeApp(quiet = false): Promise<void> {
    const closing = this.mounted;
    const opened = this.state.opened;
    const connection = this.state.connectionId;
    if (!closing && !this.state.appOpen) return;
    this.mounted = null;
    this.set({ appOpen: false });
    try {
      await closing?.teardown();
    } catch {
      // already gone
    }
    if (opened && connection) {
      await this.api("POST", `/api/connections/${connection}/apps/${opened.app_id}/close`, {}).catch(() => {});
    }
    if (!quiet) this.set({ appsStatus: "app closed" });
  }
}
