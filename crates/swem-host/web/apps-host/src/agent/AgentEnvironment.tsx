// Configuring an agent: where it works, and what it can reach.
//
// This is the half of "personal agent" that was missing. A profile used to be
// created once with nothing attached, which meant every attempt to let the
// agent see a project refused - the agent could not reach the work it was
// asked about. Here a person says which MCP servers the agent attaches (their
// own project's cycle among them), declares servers of their own, and moves
// the directory the agent works in.

import { useEffect, useRef, useState } from "react";
import { useStore } from "zustand";

import { world } from "../page/world.ts";

import { AuthCard } from "./AuthCard.tsx";
import type { AgentSkill, DeclareMcpServer, McpServerView, ModelProviderView } from "./session.ts";

/// A skill installed from the Store, as the host reads its SKILL.md.
interface InstalledSkill {
  id: string;
  version: string;
  name: string;
  description: string;
  body: string;
}
import { sessionStore, useSession } from "./store.ts";

/// The form for a place a model is served from: an OpenAI-compatible
/// endpoint under the desk, a vendor with an account, a router.
function DeclareModelProvider({ onDone }: { onDone: () => void }) {
  const state = useSession();
  const vault = state.secrets[state.profileId];
  const kinds = vault?.types ?? [];
  const id = useRef<HTMLInputElement>(null);
  const name = useRef<HTMLInputElement>(null);
  const url = useRef<HTMLInputElement>(null);
  const models = useRef<HTMLTextAreaElement>(null);
  const [keyType, setKeyType] = useState("generic_env_var");
  const submit = async () => {
    const lines = (models.current?.value ?? "").split("\n").map((line) => line.trim()).filter(Boolean);
    const ok = await sessionStore.declareModelProvider({
      id: id.current?.value.trim() ?? "",
      name: name.current?.value.trim() ?? "",
      base_url: url.current?.value.trim() ?? "",
      key_type: keyType,
      models: lines.map((line) => {
        const [modelId, ...rest] = line.split("·").map((part) => part.trim());
        return { id: modelId ?? "", name: rest.join(" · ") };
      }),
    });
    if (ok) onDone();
  };
  return (
    <div className="declare-server" id="declare-model-provider">
      <input id="provider-id" placeholder="id (letters, digits, - and _)" ref={id} />
      <input id="provider-name" placeholder="what to call it" ref={name} />
      <input id="provider-base-url" placeholder="address (https://…/v1), or empty for the vendor's own" ref={url} />
      <select id="provider-key-type" aria-label="Kind of key" value={keyType} onChange={(event) => setKeyType(event.target.value)}>
        {kinds.map((kind) => (
          <option value={kind.type_id} key={kind.type_id}>
            {kind.label}
            {kind.env_var ? ` (${kind.env_var})` : ""}
          </option>
        ))}
        {kinds.length === 0 ? <option value="generic_env_var">Environment variable</option> : null}
      </select>
      <textarea id="provider-models" placeholder={"models it serves, one per line: id · name\n(leave empty to type any model id)"} rows={3} ref={models} />
      <button id="add-model-provider" className="primary" onClick={() => void submit()}>
        Add provider
      </button>
    </div>
  );
}

/// Which model this profile asks for, from which provider - and the key that
/// provider takes, saved where the agent's other keys are.
function ModelSection({ providers }: { providers: ModelProviderView[] }) {
  const state = useSession();
  const setup = sessionStore.profileSetup();
  const [declaring, setDeclaring] = useState(false);
  const [custom, setCustom] = useState(setup.model ?? "");
  useEffect(() => setCustom(setup.model ?? ""), [setup.model]);
  const provider = providers.find((candidate) => candidate.id === setup.model_provider) ?? null;
  const vault = state.secrets[state.profileId];
  const keyPresent = provider?.key_env ? (vault?.secrets ?? []).some((secret) => secret.name === provider.key_env) : null;
  return (
    <>
      <div className="k-eyebrow">Model</div>
      <p className="k-caption k-muted">
        The model this agent answers from, and where it is served. Left on the agent's default, the
        agent picks as it would on its own.
      </p>
      <label className="field">
        <span className="k-caption">Provider</span>
        <select
          id="profile-model-provider"
          aria-label="Model provider"
          value={setup.model_provider ?? ""}
          disabled={state.busy}
          onChange={(event) => void sessionStore.setProfileSetup({ model_provider: event.target.value || null, model: null })}
        >
          <option value="">agent's default</option>
          {providers.map((candidate) => (
            <option value={candidate.id} key={candidate.id}>
              {candidate.name}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        <span className="k-caption">Model</span>
        {provider && provider.models.length > 0 ? (
          <select
            id="profile-model"
            aria-label="Model"
            value={setup.model ?? ""}
            disabled={state.busy}
            onChange={(event) => void sessionStore.setProfileSetup({ model: event.target.value || null })}
          >
            <option value="">agent's default</option>
            {provider.models.map((choice) => (
              <option value={choice.id} key={choice.id}>
                {choice.name || choice.id}
              </option>
            ))}
          </select>
        ) : (
          <span className="pair">
            <input
              id="profile-model"
              aria-label="Model"
              placeholder="model id, as the agent names it"
              value={custom}
              onChange={(event) => setCustom(event.target.value)}
            />
            <button id="save-model" disabled={state.busy || custom.trim() === (setup.model ?? "")} onClick={() => void sessionStore.setProfileSetup({ model: custom.trim() || null })}>
              Save
            </button>
          </span>
        )}
      </label>
      {provider ? (
        <p id="provider-key" className="k-caption k-muted" data-key-present={keyPresent === null ? undefined : String(keyPresent)}>
          {provider.name} takes {provider.key_label}
          {provider.key_env ? ` (${provider.key_env})` : ""}
          {keyPresent === true ? " · in place" : keyPresent === false ? " · not yet given; add it under Keys below" : ""}
        </p>
      ) : null}
      <ul id="model-providers" className="server-list">
        {providers.map((candidate) => (
          <li className="provider-row" data-provider={candidate.id} data-origin={candidate.origin} key={candidate.id}>
            <strong>{candidate.name}</strong>
            <span className="k-chip">{candidate.origin === "built_in" ? "shipped" : "yours"}</span>
            <span className="k-caption k-muted">
              {candidate.base_url ?? "vendor's own address"} · {candidate.key_label}
            </span>
            {candidate.origin === "declared" ? (
              <button className="provider-forget" data-provider={candidate.id} onClick={() => void sessionStore.forgetModelProvider(candidate.id)}>
                Forget
              </button>
            ) : null}
          </li>
        ))}
      </ul>
      <button id="add-model-provider-open" onClick={() => setDeclaring((open) => !open)}>
        {declaring ? "Cancel" : "Add a provider"}
      </button>
      {declaring ? <DeclareModelProvider onDone={() => setDeclaring(false)} /> : null}
    </>
  );
}

/// The role, in the person's words, written into the agent's instruction
/// file at every session start.
function RoleSection() {
  const state = useSession();
  const setup = sessionStore.profileSetup();
  const [draft, setDraft] = useState(setup.role);
  useEffect(() => setDraft(setup.role), [setup.role, state.profileId]);
  return (
    <>
      <div className="k-eyebrow">Role</div>
      <p className="k-caption k-muted">
        Who this agent is and how it works, in your words. Written into its instruction file in its
        working directory at every session start; anything you wrote there yourself, below SWEM's
        marker, is kept.
      </p>
      <textarea id="profile-role" aria-label="Role" rows={5} value={draft} onChange={(event) => setDraft(event.target.value)} />
      <button id="save-role" disabled={state.busy || draft === setup.role} onClick={() => void sessionStore.setProfileSetup({ role: draft })}>
        Save role
      </button>
    </>
  );
}

/// Skills: named instructions the agent loads when they apply, written as
/// folders the agent's own skill loader reads.
function SkillsSection() {
  const state = useSession();
  const setup = sessionStore.profileSetup();
  const name = useRef<HTMLInputElement>(null);
  const description = useRef<HTMLInputElement>(null);
  const body = useRef<HTMLTextAreaElement>(null);
  // The skills installed from the Store: a profile takes a copy of one, so
  // the profile stays whole on its own and the agent reads it the same way.
  const [installed, setInstalled] = useState<InstalledSkill[]>([]);
  const [chosen, setChosen] = useState("");
  useEffect(() => {
    sessionStore
      .api<InstalledSkill[]>("GET", "/api/skills")
      .then((skills) => setInstalled(skills))
      .catch(() => setInstalled([]));
  }, [state.profileId]);
  const attach = async () => {
    const skill = installed.find((candidate) => candidate.id === chosen);
    if (!skill) return;
    const rest = setup.agent_skills.filter((existing) => existing.name !== skill.id);
    await sessionStore.setProfileSetup({
      agent_skills: [...rest, { name: skill.id, description: skill.description, body: skill.body }],
    });
  };
  const add = async () => {
    const skill: AgentSkill = {
      name: name.current?.value.trim() ?? "",
      description: description.current?.value.trim() ?? "",
      body: body.current?.value ?? "",
    };
    if (!skill.name) return;
    const rest = setup.agent_skills.filter((existing) => existing.name !== skill.name);
    if (await sessionStore.setProfileSetup({ agent_skills: [...rest, skill] })) {
      if (name.current) name.current.value = "";
      if (description.current) description.current.value = "";
      if (body.current) body.current.value = "";
    }
  };
  return (
    <>
      <div className="k-eyebrow">Skills</div>
      <p className="k-caption k-muted">
        Instructions the agent reads when they apply - a name, when to use it, what to do. Each one
        becomes a folder in the agent's skills directory.
      </p>
      <ul id="agent-skills" className="server-list">
        {setup.agent_skills.map((skill) => (
          <li className="skill-row" data-skill={skill.name} key={skill.name}>
            <strong>{skill.name}</strong>
            <span className="k-caption k-muted">{skill.description}</span>
            <button
              className="skill-remove"
              data-skill={skill.name}
              disabled={state.busy}
              onClick={() => void sessionStore.setProfileSetup({ agent_skills: setup.agent_skills.filter((existing) => existing.name !== skill.name) })}
            >
              Remove
            </button>
          </li>
        ))}
      </ul>
      {installed.length > 0 ? (
        <div className="declare-server">
          <div className="pair">
            <select id="skill-installed" aria-label="A skill installed from the Store" value={chosen} onChange={(event) => setChosen(event.target.value)}>
              <option value="">From the Store…</option>
              {installed.map((skill) => (
                <option value={skill.id} key={skill.id}>
                  {skill.name} ({skill.id} {skill.version})
                </option>
              ))}
            </select>
            <button id="skill-attach" disabled={state.busy || !chosen} onClick={() => void attach()}>
              Give it this skill
            </button>
          </div>
        </div>
      ) : null}
      <div className="declare-server">
        <input id="skill-name" placeholder="name (letters, digits, - and _)" ref={name} />
        <input id="skill-description" placeholder="when to use it" ref={description} />
        <textarea id="skill-body" placeholder="what to do" rows={4} ref={body} />
        <button id="add-skill" disabled={state.busy} onClick={() => void add()}>
          Add skill
        </button>
      </div>
    </>
  );
}

/// What a declaration says about itself, without any secret in it.
function serverSummary(server: McpServerView): string {
  const parts: string[] = [server.transport];
  if (server.command) parts.push([server.command, ...(server.args ?? [])].join(" "));
  if (server.url) parts.push(server.url);
  const names = [...(server.env_names ?? []), ...(server.header_names ?? [])];
  if (names.length > 0) parts.push(`sets ${names.join(", ")}`);
  return parts.join(" · ");
}

/// The form for a server a person runs or dials themselves.
function DeclareServer({ onDone }: { onDone: () => void }) {
  const [transport, setTransport] = useState("stdio");
  const name = useRef<HTMLInputElement>(null);
  const command = useRef<HTMLInputElement>(null);
  const args = useRef<HTMLInputElement>(null);
  const url = useRef<HTMLInputElement>(null);
  const envName = useRef<HTMLInputElement>(null);
  const envValue = useRef<HTMLInputElement>(null);
  const submit = async () => {
    const body: DeclareMcpServer = { name: name.current?.value.trim() ?? "", transport };
    if (transport === "stdio") {
      body.command = command.current?.value.trim() ?? "";
      body.args = (args.current?.value ?? "").split(/\s+/).filter(Boolean);
      const variable = envName.current?.value.trim() ?? "";
      if (variable) body.env = [{ name: variable, value: envValue.current?.value ?? "" }];
    } else {
      body.url = url.current?.value.trim() ?? "";
    }
    if (await sessionStore.declareMcpServer(body)) {
      if (name.current) name.current.value = "";
      if (envValue.current) envValue.current.value = "";
      onDone();
    }
  };
  return (
    <div className="declare-server" id="declare-server">
      <input id="server-name" placeholder="name (letters, digits, - and _)" ref={name} />
      <select id="server-transport" aria-label="How it runs" value={transport} onChange={(event) => setTransport(event.target.value)}>
        <option value="stdio">runs here (stdio)</option>
        <option value="http">over the network (http)</option>
        <option value="sse">over the network (sse)</option>
      </select>
      {transport === "stdio" ? (
        <>
          <input id="server-command" placeholder="command that starts it" ref={command} />
          <input id="server-args" placeholder="arguments, separated by spaces" ref={args} />
          <div className="pair">
            <input id="server-env-name" placeholder="variable it needs" ref={envName} />
            <input id="server-env-value" placeholder="its value" type="password" ref={envValue} />
          </div>
          <p className="k-caption k-muted">
            A value typed here is kept in this server's declaration on disk so it can be started
            again. It is never written to a record and never shown back.
          </p>
        </>
      ) : (
        <input id="server-url" placeholder="https://…" ref={url} />
      )}
      <button id="declare-server-save" className="primary" onClick={() => void submit()}>
        Declare server
      </button>
    </div>
  );
}

export function AgentEnvironment({ hidden }: { hidden: boolean }) {
  const state = useSession();
  const profile = state.profiles.find((candidate) => candidate.profile_id === state.profileId) ?? null;
  const [declaring, setDeclaring] = useState(false);
  const workspace = useRef<HTMLInputElement>(null);
  const agentHome = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (hidden) return;
    void sessionStore.loadMcpServers();
    void sessionStore.loadPermissionProfiles();
    void sessionStore.loadEnvironments();
    void sessionStore.loadModelProviders();
  }, [hidden]);
  // The agent asked to be signed in: said here, where the keys are, as well
  // as on the rail.
  const failed = useStore(world, (known) => {
    const agent = Object.values(known.participants).find((one) => one.profile_id === state.profileId);
    return agent ? (known.failed[agent.participant_id] ?? "") : "";
  });
  const needsAuthentication = failed.includes("requires authentication");
  useEffect(() => {
    if (workspace.current) workspace.current.value = profile?.workspace ?? "";
    if (agentHome.current) agentHome.current.value = profile?.agent_home ?? "";
  }, [profile?.profile_id, profile?.revision, profile?.workspace, profile?.agent_home]);
  const attached = new Set((profile?.attachments ?? []).map((attachment) => attachment.server_name));
  const toggle = async (name: string) => {
    const next = new Set(attached);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    await sessionStore.amendProfile({ attachments: [...next] });
  };
  const saveDirectories = async () => {
    // What the person typed, an emptied field included. Trimming an empty
    // field down to `undefined` meant "leave this one as it was": the host
    // never saw the change, and the page answered "Saved." to a directory it
    // had thrown away. The host refuses a path it cannot use, in words, and
    // that is the answer a person needs here.
    await sessionStore.amendProfile({
      workspace: workspace.current?.value ?? undefined,
      agent_home: agentHome.current?.value ?? undefined,
    });
  };
  return (
    <section className="environment" data-agent-panel="environment" hidden={hidden} aria-label="Agent environment">
      {profile === null ? (
        <p className="k-muted">Choose an agent to configure it.</p>
      ) : (
        <>
          <ModelSection providers={state.modelProviders} />
          <RoleSection />
          <SkillsSection />

          <div className="k-eyebrow">Tools this agent reaches</div>
          <p className="k-caption k-muted">
            An agent reaches what you work on through a tool server. Attach one here and the
            agent reads and changes the same work you do.
          </p>
          <ul id="mcp-servers" className="server-list">
            {state.mcpServers.map((server) => (
              <li className="server-row" data-server={server.name} data-origin={server.origin} key={server.name}>
                <label>
                  <input
                    type="checkbox"
                    className="server-attach"
                    data-server={server.name}
                    checked={attached.has(server.name)}
                    disabled={state.busy}
                    onChange={() => void toggle(server.name)}
                  />
                  <strong>{server.name}</strong>
                  <span className="k-chip">{server.origin === "product" ? "came with SWEM" : "declared"}</span>
                </label>
                <span className="k-caption k-muted">{serverSummary(server)}</span>
                {server.origin === "catalogue" ? (
                  <button className="server-forget" data-server={server.name} onClick={() => void sessionStore.forgetMcpServer(server.name)}>
                    Forget
                  </button>
                ) : null}
              </li>
            ))}
            {/* What the agent attaches and this computer does not have: said,
                and detachable. It is not in the list above, which holds only
                what is declared here. */}
            {state.mcpServersKnown
              ? [...attached]
                  .filter((name) => !state.mcpServers.some((server) => server.name === name))
                  .map((name) => (
                    <li className="server-row" data-server={name} data-missing="true" key={name}>
                      <strong>{name}</strong>
                      <span className="k-caption k-muted">
                        Attached, but not set up on this computer. The agent works without it.
                      </span>
                      <button className="server-forget" disabled={state.busy} onClick={() => void toggle(name)}>
                        Detach
                      </button>
                    </li>
                  ))
              : null}
          </ul>
          {state.mcpServers.length === 0 ? (
            <p id="mcp-servers-empty" className="k-muted">
              Nothing to attach yet. Install a server from the Store, or declare one below.
            </p>
          ) : null}
          <button id="declare-server-open" onClick={() => setDeclaring((open) => !open)}>
            {declaring ? "Cancel" : "Declare a server"}
          </button>
          {declaring ? <DeclareServer onDone={() => setDeclaring(false)} /> : null}

          <div className="k-eyebrow">Where it runs</div>
          <label className="field">
            <span className="k-caption">This agent runs</span>
            <select
              id="profile-environment-choice"
              aria-label="Where this agent runs"
              value={profile.environment_profile_id ?? ""}
              disabled={state.busy || state.environments.length === 0}
              onChange={(event) => void sessionStore.amendProfile({ environment: event.target.value })}
            >
              {state.environments.map((place) => (
                <option
                  value={place.environment_profile_id}
                  key={place.environment_profile_id}
                  disabled={place.available === false && place.environment_profile_id !== profile.environment_profile_id}
                >
                  {place.name}
                  {place.available === false ? " (not on this computer today)" : ""}
                </option>
              ))}
            </select>
          </label>
          {state.environments
            .filter((place) => place.available === false)
            .map((place) => (
              <p className="k-caption k-muted" key={place.environment_profile_id}>
                {place.name}: {place.why_not}
              </p>
            ))}
          <p id="profile-environment" className="k-caption" data-environment={profile.environment_profile_id}>
            {state.environments.find(
              (place) => place.environment_profile_id === profile.environment_profile_id,
            )?.name ?? `an environment this product does not have (${profile.environment_profile_id})`}
          </p>
          <p id="environment-summary" className="k-caption k-muted">
            {state.environments.find(
              (place) => place.environment_profile_id === profile.environment_profile_id,
            )?.summary ??
              "A session with this profile will be refused until it names an environment this product has."}
          </p>

          <div className="k-eyebrow">From your editor</div>
          <p className="k-caption k-muted">
            This agent also answers an editor that hosts agents. Point it at this command and you
            work with the same agent, in the same directory, without leaving your editor.
          </p>
          <p id="editor-command" className="k-caption" data-profile={profile.profile_id}>
            swem acp --profile {profile.profile_id}
          </p>

          <div className="k-eyebrow">How it asks</div>
          <p className="k-caption k-muted">
            An agent asks before it changes anything. You decide whether it asks you, or decides
            for itself inside a boundary this product enforces.
          </p>
          <label className="field">
            <span className="k-caption">When this agent needs permission</span>
            <select
              id="profile-permissions"
              aria-label="When this agent needs permission"
              value={profile.permission_profile_id ?? ""}
              disabled={state.busy || state.permissionProfiles.length === 0}
              onChange={(event) => void sessionStore.amendProfile({ permissions: event.target.value })}
            >
              {state.permissionProfiles.map((choice) => (
                <option value={choice.permission_profile_id} key={choice.permission_profile_id}>
                  {choice.name}
                </option>
              ))}
            </select>
          </label>
          <p id="permissions-summary" className="k-caption k-muted">
            {state.permissionProfiles.find(
              (choice) => choice.permission_profile_id === profile.permission_profile_id,
            )?.summary ?? ""}
          </p>

          <div className="k-eyebrow">Keys and sign-in</div>
          <p className="k-caption k-muted">
            What the agent needs to sign in, under the names it asks for, and the keys this
            profile holds. A provider's key lives here too.
          </p>
          <AuthCard methodId={state.authMethodId} onMethod={(id) => sessionStore.setAuthMethod(id)} needsAuthentication={needsAuthentication} />

          <div className="k-eyebrow">What it does on its own</div>
          <button id="schedules-refresh" onClick={() => void sessionStore.loadSchedules()}>
            Refresh
          </button>
          <p className="k-caption k-muted">
            A standing instruction the clock sends, in the same words you would type. It runs once
            as soon as you set it, then on its interval. This product holds the clock, so nothing
            runs while it is closed and nothing is caught up afterwards.
          </p>
          <ul id="schedules" className="schedule-list">
            {state.schedules.map((schedule) => (
              <li className="schedule-row" data-schedule={schedule.schedule_id} key={schedule.schedule_id}>
                <strong>{schedule.schedule_id}</strong>
                <span className="k-chip">every {schedule.every_minutes} min</span>
                <span className="k-caption">{schedule.say}</span>
                <span className="k-caption k-muted" data-schedule-outcome={schedule.schedule_id}>
                  {schedule.last_outcome ?? "not run yet"}
                </span>
                <button
                  className="schedule-forget"
                  data-schedule={schedule.schedule_id}
                  disabled={state.busy}
                  onClick={() => void sessionStore.forgetSchedule(schedule.schedule_id)}
                >
                  Forget
                </button>
              </li>
            ))}
          </ul>
          <div className="schedule-new">
            <input id="schedule-id" placeholder="what to call it (e.g. morning)" />
            <input id="schedule-say" placeholder="what to say to the agent" />
            <input id="schedule-minutes" type="number" min={1} defaultValue={60} placeholder="every N minutes" />
            <button
              id="schedule-save"
              disabled={state.busy}
              onClick={() => {
                const field = (id: string) =>
                  (document.getElementById(id) as HTMLInputElement | null)?.value.trim() ?? "";
                void sessionStore.setSchedule(
                  field("schedule-id"),
                  field("schedule-say"),
                  Number.parseInt(field("schedule-minutes"), 10) || 60,
                );
              }}
            >
              Set
            </button>
          </div>

          <div className="k-eyebrow">Where it works</div>
          <label className="field">
            <span className="k-caption">Working directory</span>
            <input id="profile-workspace" ref={workspace} defaultValue={profile.workspace ?? ""} />
          </label>
          <label className="field">
            <span className="k-caption">The agent's own home (where a sign-in is kept)</span>
            <input id="profile-agent-home" ref={agentHome} defaultValue={profile.agent_home ?? ""} />
          </label>
          <button id="save-directories" disabled={state.busy} onClick={() => void saveDirectories()}>
            Save
          </button>
          <div id="environment-status" className="k-caption">
            {state.environmentStatus}
          </div>
          <p className="k-caption k-muted">
            A session already running keeps the configuration it started with. Start a new session
            to use what you just changed.
          </p>
        </>
      )}
    </section>
  );
}
