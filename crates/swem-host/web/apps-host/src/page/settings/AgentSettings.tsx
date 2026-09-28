// An agent's settings: what it is put together from, who it is, where it
// works, what it reaches, what it knows how to do, how it asks. What it is
// told on time is its Schedules, a tab of its own.
//
// Each part is a card of its own and says for itself what became of what
// was done in it. What is chosen from a list is taken when it is chosen;
// what is written is taken when the person says so.

import { useEffect, useState, type ReactNode } from "react";
import { useForm } from "react-hook-form";
import { useStore } from "zustand";

import type { AgentSkill, DeclareMcpServer, McpServerView, Profile } from "../../agent/session.ts";
import { sessionStore, useSession } from "../../agent/store.ts";
import { fetchJson } from "../../http.ts";
import { Box, Brain, Laptop, Wrench } from "../icons.tsx";
import { go } from "../place.ts";
import { providers as providersStore, providing } from "../providers.ts";
import type { Participant } from "../types.ts";
import { act, world } from "../world.ts";
import { inPart, Said, type Part } from "./said.tsx";
import { SignIn } from "./SignIn.tsx";

const PARTS: { part: Part; label: string }[] = [
  { part: "together", label: "Put together" },
  { part: "identity", label: "Identity" },
  { part: "place", label: "Where it works" },
  { part: "tools", label: "Tools" },
  { part: "skills", label: "Skills" },
  { part: "permissions", label: "Permissions" },
  { part: "signin", label: "Signing in" },
];

function Card({ part, title, about, children }: { part: Part; title: string; about?: string; children: ReactNode }) {
  return (
    <section className="k-card k-stack w-part" id={`settings-${part}`} aria-label={title}>
      <div className="w-col w-close">
        <h2 className="k-heading">{title}</h2>
        {about ? <span className="k-caption">{about}</span> : null}
      </div>
      {children}
    </section>
  );
}

function Field({ label, note, children }: { label: string; note?: string; children: ReactNode }) {
  return (
    <label className="k-stack w-close">
      <span className="k-caption">{label}</span>
      {children}
      {note ? <span className="k-caption">{note}</span> : null}
    </label>
  );
}

function Standing({ tone, children }: { tone: "ready" | "asking" | "none"; children: string }) {
  return (
    <span className="k-status">
      <span className={`k-dot k-small${tone === "none" ? "" : ` k-${tone}`}`} />
      <span>{children}</span>
    </span>
  );
}

/// What the agent stands on: its engine, the model it answers from, the
/// place it lives.
function Together({ profile, agent }: { profile: Profile; agent: Participant }) {
  const state = useSession();
  const standing = useStore(providersStore, (known) => known.models);
  const [changing, setChanging] = useState<"model" | "host" | null>(null);
  const setup = sessionStore.profileSetup();
  const [custom, setCustom] = useState(setup.model ?? "");
  useEffect(() => setCustom(setup.model ?? ""), [setup.model]);
  useEffect(() => {
    void providing.models();
  }, [profile.revision]);
  const engine = state.onboarding?.agents.find((one) => one.agent_id === profile.agent_id);
  const provider = (standing ?? []).find((one) => one.id === setup.model_provider) ?? null;
  const model = provider?.models.find((one) => one.id === setup.model)?.name || setup.model || "";
  const ownKey = provider?.key_env ? (state.secrets[state.profileId]?.secrets ?? []).some((secret) => secret.name === provider.key_env) : false;
  const opened = ownKey ? "a key of its own" : provider?.key ? "the key given in Providers" : "its engine's own sign-in";
  const host = state.environments.find((place) => place.environment_profile_id === profile.environment_profile_id);
  const take = (changes: Parameters<typeof sessionStore.setProfileSetup>[0]) => {
    inPart("together");
    return sessionStore.setProfileSetup(changes);
  };
  return (
    <Card part="together" title={`${agent.name} is put together from`} about="Each part is set up once in Providers and can serve any number of agents.">
      <div>
        <div className="k-row w-nowrap">
          <Wrench size={18} />
          <span className="k-eyebrow w-part-name">Engine</span>
          <span className="k-two k-grow">
            <span className="k-name">{engine?.name ?? profile.agent_id}</span>
            <span className="k-caption">The coding agent it stands on</span>
          </span>
          <Standing tone={engine?.available === false ? "asking" : "ready"}>{engine?.available === false ? "Not on this computer" : "Installed"}</Standing>
        </div>
        <div className="k-row w-nowrap">
          <Brain size={18} />
          <span className="k-eyebrow w-part-name">Model</span>
          <span className="k-two k-grow">
            <span className="k-name">{model || "Its engine's own choice"}</span>
            <span className="k-caption">{provider ? `${provider.name} · opened by ${opened}` : "No provider chosen: it answers as its engine would by itself"}</span>
          </span>
          <button type="button" className="k-btn k-quiet" aria-expanded={changing === "model"} onClick={() => setChanging(changing === "model" ? null : "model")}>
            {changing === "model" ? "Done" : "Change"}
          </button>
        </div>
        {changing === "model" ? (
          <div className="k-stack w-change">
            <Field label="Provider">
              <select
                className="k-field"
                id="profile-model-provider"
                value={setup.model_provider ?? ""}
                disabled={state.busy}
                onChange={(event) => void take({ model_provider: event.target.value || null, model: null })}
              >
                <option value="">Its engine's own</option>
                {(standing ?? []).map((candidate) => (
                  <option value={candidate.id} key={candidate.id}>
                    {candidate.name}
                  </option>
                ))}
              </select>
            </Field>
            {provider && provider.models.length > 0 ? (
              <Field label="Model">
                <select className="k-field" id="profile-model" value={setup.model ?? ""} disabled={state.busy} onChange={(event) => void take({ model: event.target.value || null })}>
                  <option value="">Its engine's own choice</option>
                  {provider.models.map((choice) => (
                    <option value={choice.id} key={choice.id}>
                      {choice.name || choice.id}
                    </option>
                  ))}
                </select>
              </Field>
            ) : (
              <form
                className="k-stack w-close"
                onSubmit={(event) => {
                  event.preventDefault();
                  void take({ model: custom.trim() || null });
                }}
              >
                <Field label="Model" note="As its engine names it.">
                  <span className="k-inline w-tight w-nowrap">
                    <input className="k-field k-grow k-mono" id="profile-model" value={custom} spellCheck={false} onChange={(event) => setCustom(event.target.value)} />
                    <button type="submit" id="save-model" className="k-btn" disabled={state.busy || custom.trim() === (setup.model ?? "")}>
                      Save
                    </button>
                  </span>
                </Field>
              </form>
            )}
            {provider ? (
              <span id="provider-key" className="k-caption" data-key-present={String(Boolean(provider.key) || ownKey)}>
                {ownKey
                  ? `This agent holds a key of its own for ${provider.name}. `
                  : provider.key
                    ? `${provider.name} was given what opens it, for every agent. `
                    : `${provider.name} was given nothing here, so this agent signs in the way its engine does. `}
                <a href="#/providers/models" onClick={() => go({ at: "providers", tab: "models" })}>
                  Providers
                </a>
              </span>
            ) : null}
          </div>
        ) : null}
        <div className="k-row w-nowrap">
          {profile.environment_profile_id === "podman-container-environment" ? <Box size={18} /> : <Laptop size={18} />}
          <span className="k-eyebrow w-part-name">Host</span>
          <span className="k-two k-grow">
            <span id="profile-environment" className="k-name" data-environment={profile.environment_profile_id}>
              {host?.name ?? `A place this product does not have (${profile.environment_profile_id})`}
            </span>
            <span className="k-caption">Where it lives: its files, its terminal and everything it starts</span>
          </span>
          <Standing tone={host && host.available !== false ? "ready" : "asking"}>{!host ? "Unknown" : host.available === false ? "Cannot start" : "Ready"}</Standing>
          <button type="button" className="k-btn k-quiet" aria-expanded={changing === "host"} onClick={() => setChanging(changing === "host" ? null : "host")}>
            {changing === "host" ? "Done" : "Change"}
          </button>
        </div>
        {changing === "host" ? (
          <div className="k-stack w-change">
            <Field label="This agent lives">
              <select
                className="k-field"
                id="profile-environment-choice"
                value={profile.environment_profile_id ?? ""}
                disabled={state.busy || state.environments.length === 0}
                onChange={(event) => {
                  inPart("together");
                  void sessionStore.amendProfile({ environment: event.target.value });
                }}
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
            </Field>
            {state.environments
              .filter((place) => place.available === false)
              .map((place) => (
                <span className="k-caption" key={place.environment_profile_id}>
                  {place.name}: {place.why_not}
                </span>
              ))}
          </div>
        ) : null}
      </div>
      <span id="environment-summary" className="k-caption">
        {host?.summary ?? "It cannot answer until it is put somewhere this product has."}
      </span>
      {host ? null : <div className="k-notice k-danger">It cannot answer until it is put somewhere this product has.</div>}
      <Said part="together" />
    </Card>
  );
}

interface Who {
  name: string;
  handle: string;
  role: string;
}

/// Who the agent is: what it is called, the handle it is named by, and
/// what it is for.
function Identity({ profile, agent }: { profile: Profile; agent: Participant }) {
  const state = useSession();
  const [problem, setProblem] = useState("");
  const form = useForm<Who>({ values: { name: agent.name, handle: agent.handle, role: profile.role ?? "" } });
  const save = form.handleSubmit(async (who) => {
    setProblem("");
    inPart("identity");
    try {
      if (who.name.trim() !== agent.name || who.handle.trim() !== agent.handle) {
        await fetchJson(`/api/people/${encodeURIComponent(agent.participant_id)}`, {
          method: "PATCH",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ name: who.name.trim(), handle: who.handle.trim().replace(/^@/, "") }),
        });
        await act.people();
      }
      // The words it is for are taken even when they are the same: the
      // person pressed Save and is told it was saved.
      await sessionStore.setProfileSetup({ role: who.role });
    } catch (error) {
      setProblem((error as Error).message);
    }
  });
  return (
    <Card part="identity" title="Identity">
      <form className="k-stack" onSubmit={(event) => void save(event)}>
        <div className="w-pair">
          <Field label="Name">
            <input className="k-field" {...form.register("name", { required: true, maxLength: 96 })} />
          </Field>
          <Field label="Handle" note="What it is named by in a chat, after @.">
            <input className="k-field k-mono" spellCheck={false} {...form.register("handle", { required: true, maxLength: 32 })} />
          </Field>
        </div>
        <Field label="What it is for" note={`Becomes ${agent.name}'s standing instructions, written where its engine reads them each time it starts. What you wrote there yourself is kept.`}>
          <textarea className="k-field" id="profile-role" rows={5} {...form.register("role")} />
        </Field>
        <div className="k-inline w-tight">
          <button type="submit" id="save-role" className="k-btn k-primary" disabled={state.busy || !form.formState.isDirty}>
            Save
          </button>
          {problem ? <span className="k-caption k-is-danger">{problem}</span> : <Said part="identity" />}
        </div>
      </form>
    </Card>
  );
}

interface Places {
  workspace: string;
  agent_home: string;
}

function Place({ profile }: { profile: Profile }) {
  const state = useSession();
  const form = useForm<Places>({ values: { workspace: profile.workspace ?? "", agent_home: profile.agent_home ?? "" } });
  const save = form.handleSubmit(async (places) => {
    inPart("place");
    // What was typed, an emptied field included: the host refuses a path
    // it cannot use, in words, and that is the answer a person needs.
    await sessionStore.amendProfile({ workspace: places.workspace, agent_home: places.agent_home });
  });
  return (
    <Card part="place" title="Where it works" about="Two folders on its host: the one it works in, and its own home, where its engine keeps a sign-in.">
      <form className="k-stack" onSubmit={(event) => void save(event)}>
        <Field label="The folder it works in">
          <input className="k-field k-mono" id="profile-workspace" spellCheck={false} {...form.register("workspace")} />
        </Field>
        <Field label="Its own home">
          <input className="k-field k-mono" id="profile-agent-home" spellCheck={false} {...form.register("agent_home")} />
        </Field>
        <div className="k-inline w-tight">
          <button type="submit" id="save-directories" className="k-btn k-primary" disabled={state.busy}>
            Save
          </button>
          <Said part="place" />
        </div>
      </form>
      <div className="k-stack w-close">
        <span className="k-eyebrow">From your editor</span>
        <span className="k-caption">An editor that hosts agents works with this same agent, in the same folder, when it is pointed at this command.</span>
        <code id="editor-command" className="k-code" data-profile={profile.profile_id}>
          swem acp --profile {profile.profile_id}
        </code>
      </div>
    </Card>
  );
}

/// What a declaration says about itself, without any secret in it.
function serverSummary(server: McpServerView): string {
  const parts: string[] = [];
  if (server.command) parts.push([server.command, ...(server.args ?? [])].join(" "));
  if (server.url) parts.push(server.url);
  const names = [...(server.env_names ?? []), ...(server.header_names ?? [])];
  if (names.length > 0) parts.push(`sets ${names.join(", ")}`);
  return parts.join(" · ");
}

interface Server {
  name: string;
  transport: string;
  command: string;
  args: string;
  env_name: string;
  env_value: string;
  url: string;
}

function DeclareServer({ onDone }: { onDone: () => void }) {
  const form = useForm<Server>({ defaultValues: { name: "", transport: "stdio", command: "", args: "", env_name: "", env_value: "", url: "" } });
  const transport = form.watch("transport");
  const declare = form.handleSubmit(async (server) => {
    inPart("tools");
    const body: DeclareMcpServer = { name: server.name.trim(), transport: server.transport };
    if (server.transport === "stdio") {
      body.command = server.command.trim();
      body.args = server.args.split(/\s+/).filter(Boolean);
      if (server.env_name.trim()) body.env = [{ name: server.env_name.trim(), value: server.env_value }];
    } else {
      body.url = server.url.trim();
    }
    if (await sessionStore.declareMcpServer(body)) onDone();
  });
  return (
    <form className="k-card k-stack w-inset" id="declare-server" onSubmit={(event) => void declare(event)}>
      <Field label="What it is called" note="Letters, digits, dashes and underscores.">
        <input className="k-field" id="server-name" spellCheck={false} {...form.register("name")} />
      </Field>
      <Field label="How it is reached">
        <select className="k-field" id="server-transport" {...form.register("transport")}>
          <option value="stdio">It is started on this computer</option>
          <option value="http">Over the network</option>
          <option value="sse">Over the network, as a stream of events</option>
        </select>
      </Field>
      {transport === "stdio" ? (
        <>
          <Field label="The command that starts it">
            <input className="k-field k-mono" id="server-command" spellCheck={false} {...form.register("command")} />
          </Field>
          <Field label="What the command is given" note="Separated by spaces.">
            <input className="k-field k-mono" id="server-args" spellCheck={false} {...form.register("args")} />
          </Field>
          <div className="w-pair">
            <Field label="A variable it needs">
              <input className="k-field k-mono" id="server-env-name" spellCheck={false} {...form.register("env_name")} />
            </Field>
            <Field label="Its value" note="Kept with this server on this computer so it can be started again. Never shown back.">
              <input className="k-field" id="server-env-value" type="password" autoComplete="off" {...form.register("env_value")} />
            </Field>
          </div>
        </>
      ) : (
        <Field label="Its address">
          <input className="k-field k-mono" id="server-url" placeholder="https://…" spellCheck={false} {...form.register("url")} />
        </Field>
      )}
      <div className="k-inline w-tight">
        <button type="submit" id="declare-server-save" className="k-btn k-primary">
          Add the server
        </button>
        <button type="button" className="k-btn k-quiet" onClick={onDone}>
          Not now
        </button>
      </div>
    </form>
  );
}

function Tools({ profile }: { profile: Profile }) {
  const state = useSession();
  const [declaring, setDeclaring] = useState(false);
  const attached = new Set((profile.attachments ?? []).map((attachment) => attachment.server_name));
  const toggle = async (name: string) => {
    inPart("tools");
    const next = new Set(attached);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    await sessionStore.amendProfile({ attachments: [...next] });
  };
  return (
    <Card part="tools" title="Tools" about="Servers this agent reaches. What you work on through one, the agent reads and changes through the same one.">
      <div id="mcp-servers">
        {state.mcpServers.map((server) => (
          <div className="server-row k-row w-nowrap" data-server={server.name} data-origin={server.origin} key={server.name}>
            <label className="k-inline w-tight k-grow w-nowrap">
              <input
                type="checkbox"
                className="server-attach"
                data-server={server.name}
                checked={attached.has(server.name)}
                disabled={state.busy}
                onChange={() => void toggle(server.name)}
              />
              <span className="k-two">
                <span className="k-name">{server.name}</span>
                <span className="k-caption">{[server.origin === "product" ? "Came with SWEM" : "Added by you", serverSummary(server)].filter(Boolean).join(" · ")}</span>
              </span>
            </label>
            {server.origin === "catalogue" ? (
              <button
                type="button"
                className="k-btn k-quiet server-forget"
                data-server={server.name}
                onClick={() => {
                  inPart("tools");
                  void sessionStore.forgetMcpServer(server.name);
                }}
              >
                Forget
              </button>
            ) : null}
          </div>
        ))}
        {/* What the agent attaches and this computer does not have: said,
            and detachable. */}
        {state.mcpServersKnown
          ? [...attached]
              .filter((name) => !state.mcpServers.some((server) => server.name === name))
              .map((name) => (
                <div className="server-row k-row w-nowrap" data-server={name} data-missing="true" key={name}>
                  <span className="k-two k-grow">
                    <span className="k-name">{name}</span>
                    <span className="k-caption">Attached, but not set up on this computer. The agent works without it.</span>
                  </span>
                  <button type="button" className="k-btn k-quiet server-forget" disabled={state.busy} onClick={() => void toggle(name)}>
                    Detach
                  </button>
                </div>
              ))
          : null}
      </div>
      {state.mcpServers.length === 0 ? (
        <span id="mcp-servers-empty" className="k-caption">
          Nothing to attach yet. Install a server from the Store, or add one of your own.
        </span>
      ) : null}
      {declaring ? (
        <DeclareServer onDone={() => setDeclaring(false)} />
      ) : (
        <div className="k-inline w-tight">
          <button type="button" id="declare-server-open" className="k-btn" onClick={() => setDeclaring(true)}>
            Add a server of your own
          </button>
          <Said part="tools" />
        </div>
      )}
      {declaring ? <Said part="tools" /> : null}
    </Card>
  );
}

/// A skill installed from the Store, as the host reads it.
interface InstalledSkill {
  id: string;
  version: string;
  name: string;
  description: string;
  body: string;
}

function Skills({ profile }: { profile: Profile }) {
  const state = useSession();
  const skills = profile.agent_skills ?? [];
  const [installed, setInstalled] = useState<InstalledSkill[]>([]);
  const [chosen, setChosen] = useState("");
  const form = useForm<AgentSkill>({ defaultValues: { name: "", description: "", body: "" } });
  useEffect(() => {
    fetchJson<InstalledSkill[]>("/api/skills")
      .then(setInstalled)
      .catch(() => setInstalled([]));
  }, [profile.profile_id]);
  const give = (next: AgentSkill[]) => {
    inPart("skills");
    return sessionStore.setProfileSetup({ agent_skills: next });
  };
  const without = (name: string) => skills.filter((existing) => existing.name !== name);
  const attach = async () => {
    const skill = installed.find((candidate) => candidate.id === chosen);
    if (skill) await give([...without(skill.id), { name: skill.id, description: skill.description, body: skill.body }]);
  };
  const add = form.handleSubmit(async (skill) => {
    const named = { name: skill.name.trim(), description: skill.description.trim(), body: skill.body };
    if (named.name && (await give([...without(named.name), named]))) form.reset();
  });
  return (
    <Card part="skills" title="Skills" about="What it knows how to do when it applies: a name, when to use it, what to do. Each is a folder its engine reads.">
      <div id="agent-skills">
        {skills.map((skill) => (
          <div className="skill-row k-row w-nowrap" data-skill={skill.name} key={skill.name}>
            <span className="k-two k-grow">
              <span className="k-name">{skill.name}</span>
              <span className="k-caption">{skill.description}</span>
            </span>
            <button type="button" className="k-btn k-quiet skill-remove" data-skill={skill.name} disabled={state.busy} onClick={() => void give(without(skill.name))}>
              Take away
            </button>
          </div>
        ))}
        {skills.length === 0 ? <span className="k-caption">None yet.</span> : null}
      </div>
      {installed.length > 0 ? (
        <div className="k-inline w-tight w-nowrap">
          <select className="k-field k-grow" id="skill-installed" aria-label="A skill installed from the Store" value={chosen} onChange={(event) => setChosen(event.target.value)}>
            <option value="">From the Store…</option>
            {installed.map((skill) => (
              <option value={skill.id} key={skill.id}>
                {skill.name} ({skill.id} {skill.version})
              </option>
            ))}
          </select>
          <button type="button" id="skill-attach" className="k-btn" disabled={state.busy || !chosen} onClick={() => void attach()}>
            Give it this skill
          </button>
        </div>
      ) : null}
      <form className="k-card k-stack w-inset" onSubmit={(event) => void add(event)}>
        <span className="k-eyebrow">A skill of your own</span>
        <div className="w-pair">
          <Field label="Name" note="Letters, digits, dashes and underscores.">
            <input className="k-field" id="skill-name" spellCheck={false} {...form.register("name")} />
          </Field>
          <Field label="When to use it">
            <input className="k-field" id="skill-description" {...form.register("description")} />
          </Field>
        </div>
        <Field label="What to do">
          <textarea className="k-field" id="skill-body" rows={4} {...form.register("body")} />
        </Field>
        <div className="k-inline w-tight">
          <button type="submit" id="add-skill" className="k-btn" disabled={state.busy}>
            Add the skill
          </button>
          <Said part="skills" />
        </div>
      </form>
    </Card>
  );
}

function Permissions({ profile }: { profile: Profile }) {
  const state = useSession();
  const chosen = state.permissionProfiles.find((choice) => choice.permission_profile_id === profile.permission_profile_id);
  return (
    <Card part="permissions" title="Permissions" about="An agent asks before it changes anything. Either it asks you, or it decides for itself inside a boundary this product holds it to.">
      <Field label="When it needs permission">
        <select
          className="k-field"
          id="profile-permissions"
          value={profile.permission_profile_id ?? ""}
          disabled={state.busy || state.permissionProfiles.length === 0}
          onChange={(event) => {
            inPart("permissions");
            void sessionStore.amendProfile({ permissions: event.target.value });
          }}
        >
          {state.permissionProfiles.map((choice) => (
            <option value={choice.permission_profile_id} key={choice.permission_profile_id}>
              {choice.name}
            </option>
          ))}
        </select>
      </Field>
      <span id="permissions-summary" className="k-caption">
        {chosen?.summary ?? ""}
      </span>
      <Said part="permissions" />
    </Card>
  );
}

export function AgentSettings({ agent, hidden }: { agent: Participant; hidden: boolean }) {
  const state = useSession();
  const profile = state.profiles.find((candidate) => candidate.profile_id === state.profileId) ?? null;
  useEffect(() => {
    if (hidden) return;
    void sessionStore.loadMcpServers();
    void sessionStore.loadPermissionProfiles();
    void sessionStore.loadEnvironments();
  }, [hidden]);
  // The agent's engine asked to be signed in: said where signing in is.
  const failed = useStore(world, (known) => known.failed[agent.participant_id] ?? "");
  const needsSignIn = failed.includes("requires authentication");
  const to = (part: Part) => document.getElementById(`settings-${part}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
  return (
    <section className="w-settings" data-agent-panel="environment" hidden={hidden} aria-label={`Settings of ${agent.name}`}>
      {profile === null || profile.profile_id !== agent.profile_id ? (
        <span className="k-caption">Reading its settings…</span>
      ) : (
        <>
          <nav className="w-subnav" aria-label={`Settings of ${agent.name}`}>
            {PARTS.map((one) => (
              <button type="button" className="k-rail-item k-quiet" key={one.part} onClick={() => to(one.part)}>
                {one.label}
              </button>
            ))}
          </nav>
          <div className="w-parts" key={profile.profile_id}>
            <Together profile={profile} agent={agent} />
            <Identity profile={profile} agent={agent} />
            <Place profile={profile} />
            <Tools profile={profile} />
            <Skills profile={profile} />
            <Permissions profile={profile} />
            <Card part="signin" title="Signing in" about="How its engine is let in where it answers from.">
              <SignIn needsSignIn={needsSignIn} />
            </Card>
            <span className="k-caption">What is changed here is what it starts with the next time it starts.</span>
          </div>
        </>
      )}
    </section>
  );
}
