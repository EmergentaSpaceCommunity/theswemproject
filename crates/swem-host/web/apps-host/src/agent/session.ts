// What an agent is set up with, as the page edits it: its profile, the
// providers and servers it names, its keys and its files.
//
// What is said in chats is not here; that is `page/world.ts`. `fetch` is
// injected so a test drives this with a fake host and asks what a person
// would see.

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
  /// Whether it can be chosen today, from what was found when this
  /// computer was looked at, and why not.
  available?: boolean;
  why_not?: string;
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

/// Where a permission question's words came from: the agent reported them,
/// or the host is about to carry out exactly what it wrote.
export type PermissionProvenance = "correlated_agent_report" | "uncorrelated_agent_report" | "host_callback";

export type StructuredPhase = "acp_form" | "acp_url" | "launch" | "elicitation";

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
  /// Every place a model is served from that a profile here may name.
  modelProviders: ModelProviderView[];
  /// What an App last said a person is looking at, kept by the page so a
  /// session opened afterwards is given it too; `null` once let go of.
  offered: ModelContext | null;
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
  /// What the selected profile's agent has been handed and has handed back.
  files: HandedFile[];
  /// Why the last read of them failed, if it did. An empty list and a list
  /// that could not be read are different things to a person waiting for a
  /// file, and the panel must not say the first when it means the second.
  filesStatus: string;
  /// Every MCP server this host can attach to an agent.
  mcpServers: McpServerView[];
  /// Whether the host has said what it declares yet: until it has, a server
  /// the agent attaches cannot be called missing.
  mcpServersKnown: boolean;
  permissionProfiles: PermissionProfileOption[];
  environments: EnvironmentProfileOption[];
  /// A sentence about the last thing that happened while configuring, or "".
  environmentStatus: string;
  busy: boolean;
}

export type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export class SessionStore {
  private state: SessionState;
  private readonly listeners = new Set<() => void>();
  private readonly fetchImpl: Fetch;
  private readonly storage: Storage | null;

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
      files: [],
      filesStatus: "",
      mcpServers: [],
      mcpServersKnown: false,
      permissionProfiles: [],
      environments: [],
      environmentStatus: "",
      modelProviders: [],
      authMethodId: "",
      offered: null,
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
    });
    if (profileId) this.remember("swem.workbench.profile", profileId);
    // The agents present on this computer are worth knowing even once a
    // profile exists: a person adds a second profile of one of them from the
    // rail, without going back through first run.
    await this.loadOnboarding();
    if (profileId) void this.loadHandshake(profileId);
  }

  selectProfile(profileId: string): void {
    this.set({ profileId });
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

  /// An App of `server` said what a person is looking at
  /// (`ui/update-model-context`). Each update replaces the one before; one
  /// that says nothing lets go. The open session is given it with its next
  /// turn; without a session the page keeps it for the one opened next.
  async offerContext(server: string, update: { content?: unknown[] }): Promise<void> {
    const content = contextBlocks(update.content);
    if (content.length === 0) {
      if (this.state.offered?.server_name === server) this.clearContext();
      return;
    }
    this.set({ offered: { server_name: server, content } });
  }

  /// Let go of what an App said: what is said next goes without it.
  clearContext(): void {
    this.set({ offered: null });
  }

  /// Which of the agent's sign-in methods the next start uses.
  setAuthMethod(authMethodId: string): void {
    if (authMethodId !== this.state.authMethodId) this.set({ authMethodId });
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
  async installAgent(agentId: string, confirm: (question: string) => boolean | Promise<boolean>, thenMake = true): Promise<void> {
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
      !(await confirm(
        `Install ${plan.registry_id} ${plan.version}?\n\n` +
          `It fetches ${what}, described by ${plan.registry_index ?? "the agent registry"}, ` +
          "into this product's agent home. Nothing else on this machine changes.",
      ))
    ) {
      this.set({ onboardingStatus: "" });
      return;
    }
    this.set({ onboardingStatus: `Installing ${plan.registry_id}…` });
    try {
      await this.api("POST", "/api/agents/install", { agent_id: agentId, plan_id: plan.plan_id });
      await this.loadOnboarding();
      // An agent is made of what was installed at once, or by whoever
      // asked for it to be installed, in their own steps.
      if (thenMake) {
        this.set({ onboardingStatus: "Installed. Creating local profile…" });
        await this.useAgent(agentId);
      } else {
        this.set({ onboardingStatus: "" });
      }
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

  async loadHandshake(profileId: string): Promise<void> {
    if (!profileId) return;
    void this.loadFiles(profileId);
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

}
