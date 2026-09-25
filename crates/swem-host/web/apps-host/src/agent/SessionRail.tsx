// The left rail: which agent, how it is signed in, and the session's state.

import { useEffect, useRef, useState } from "react";

import type { Phase } from "./session.ts";
import { sessionStore, useSession } from "./store.ts";

function phaseWords(phase: Phase | null, connected: boolean, problem: string): string {
  if (!phase) return "not connected";
  switch (phase.phase) {
    case "connecting":
      return "starting…";
    case "idle":
      return "ready";
    case "prompting":
      return phase.turn_index === undefined ? "working…" : `working (turn ${phase.turn_index})…`;
    case "finished":
      return connected || !problem ? "finished" : "failed";
  }
}

/// A second profile of an agent this computer already has. One installed
/// agent, several people: each profile keeps its own workspace, its own
/// native home, its own permission choice and its own sessions, so the rail's
/// select stops meaning "which agent" and starts meaning "whose".
/// The agent behind a profile, as the product names it elsewhere. The
/// onboarding list is where the names live, and the form for adding a profile
/// already uses them; this select showed the id twice instead, so a person
/// read `ada (hands)` where the same agent is called "My own agent" two
/// inches away. An agent the list does not carry keeps its id, which is at
/// least true.
function agentName(
  agents: { agent_id: string; name: string }[] | undefined,
  agentId: string,
): string {
  return agents?.find((agent) => agent.agent_id === agentId)?.name ?? agentId;
}

function AddProfile() {
  const state = useSession();
  const [open, setOpen] = useState(false);
  const [more, setMore] = useState(false);
  const [name, setName] = useState("");
  const [agentId, setAgentId] = useState("");
  const [providerId, setProviderId] = useState("");
  const [model, setModel] = useState("");
  const [role, setRole] = useState("");
  const available = (state.onboarding?.agents ?? []).filter((agent) => agent.available);
  useEffect(() => {
    if (open && more && state.modelProviders.length === 0) void sessionStore.loadModelProviders();
  }, [open, more, state.modelProviders.length]);
  if (state.profiles.length === 0 || available.length === 0) return null;
  const chosen = agentId || (available[0]?.agent_id ?? "");
  const provider = state.modelProviders.find((candidate) => candidate.id === providerId) ?? null;
  if (!open) {
    return (
      <button id="add-profile-open" className="k-pick add-profile-open" onClick={() => setOpen(true)}>
        Add another profile
      </button>
    );
  }
  return (
    <div id="add-profile-form" className="add-profile">
      <input
        id="new-profile-id"
        aria-label="Profile name"
        placeholder="whose profile (e.g. ada)"
        value={name}
        onChange={(event) => setName(event.target.value)}
      />
      <select
        id="new-profile-agent"
        aria-label="Agent"
        value={chosen}
        onChange={(event) => setAgentId(event.target.value)}
      >
        {available.map((agent) => (
          <option value={agent.agent_id} key={agent.agent_id}>
            {agent.name}
          </option>
        ))}
      </select>
      {/* The model and the role can be given with the name; left out, the
          agent's own defaults stand and both can be set later in Setup. */}
      <button id="new-profile-more" className="k-pick" aria-expanded={more} onClick={() => setMore((shown) => !shown)}>
        {more ? "Less" : "More…"}
      </button>
      {more ? (
        <>
          <select id="new-profile-provider" aria-label="Model provider" value={providerId} onChange={(event) => { setProviderId(event.target.value); setModel(""); }}>
            <option value="">agent's default provider</option>
            {state.modelProviders.map((candidate) => (
              <option value={candidate.id} key={candidate.id}>
                {candidate.name}
              </option>
            ))}
          </select>
          {provider && provider.models.length > 0 ? (
            <select id="new-profile-model" aria-label="Model" value={model} onChange={(event) => setModel(event.target.value)}>
              <option value="">agent's default model</option>
              {provider.models.map((choice) => (
                <option value={choice.id} key={choice.id}>
                  {choice.name || choice.id}
                </option>
              ))}
            </select>
          ) : (
            <input id="new-profile-model" aria-label="Model" placeholder="model id (optional)" value={model} onChange={(event) => setModel(event.target.value)} />
          )}
          <textarea id="new-profile-role" aria-label="Role" placeholder="its role, in your words (optional)" rows={3} value={role} onChange={(event) => setRole(event.target.value)} />
        </>
      ) : null}
      <button
        id="add-profile"
        className="primary"
        disabled={state.busy || name.trim() === ""}
        onClick={() => {
          void sessionStore
            .useAgent(chosen, name.trim(), { model_provider: providerId || null, model: model.trim() || null, role })
            .then(() => {
              setName("");
              setRole("");
              setModel("");
              setOpen(false);
            });
        }}
      >
        Add
      </button>
      <div id="add-profile-status" className="k-caption">
        {state.onboardingStatus}
      </div>
    </div>
  );
}

/// The sessions this agent already has. A person picks one and continues it,
/// instead of being expected to remember a route id.
function Sessions() {
  const state = useSession();
  const sessions = state.sessions;
  if (state.profiles.length === 0) return null;
  return (
    <section id="sessions" className="sessions" aria-label="Sessions">
      <div className="k-eyebrow">Sessions</div>
      {sessions.length === 0 ? (
        <p id="sessions-empty" className="k-muted">
          None yet. Starting one opens a lane that is kept, so you can come back to it.
        </p>
      ) : null}
      {sessions.map((session) => (
        <button
          className={`session-row k-pick${session.route_id === state.routeId ? " k-active active" : ""}`}
          data-route-id={session.route_id}
          key={session.route_id}
          disabled={state.busy}
          onClick={() => void sessionStore.resumeSession(session.route_id)}
          title={session.route_id}
        >
          {/* What was said, not what it is called inside: a row reading
              `route-6919-1789833696034060189` is a developer's id put in front
              of a person who is trying to find the conversation they had. The
              id stays on the row's tooltip, where it is still findable. */}
          <span className="session-name">{session.opened_with ?? "Nothing said yet"}</span>
          {/* What the last record was called is the lane's own vocabulary
              (`session_new`, `agent_message_chunk`): a person reading this
              row is looking for a conversation, and that word tells them
              nothing about which one it is. The count stays, because how
              much is in a lane is something they can act on. */}
          <span className="k-caption">
            {session.events === 0 ? "a lane waiting for its first turn" : `${session.events} recorded`}
            {session.route_id === state.routeId ? " · open" : ""}
          </span>
        </button>
      ))}
    </section>
  );
}

/// The selected profile, as a card: whose agent it is, what it runs on, the
/// first line of its role, and whether a session is open on it right now.
function ProfileCard({ connected, phase }: { connected: boolean; phase: string }) {
  const state = useSession();
  const profile = state.profiles.find((candidate) => candidate.profile_id === state.profileId) ?? null;
  if (!profile) return null;
  const role = (profile.role ?? "").split("\n").find((line) => line.trim()) ?? "";
  return (
    <div id="profile-card" className="profile-card k-card" data-profile={profile.profile_id}>
      <div className="profile-card-head">
        <span id="status-dot" className={connected ? "status-dot k-dot k-live" : "status-dot k-dot"} />
        <strong>{profile.profile_id}</strong>
        <span className="k-muted">{agentName(state.onboarding?.agents, profile.agent_id)}</span>
      </div>
      <div className="profile-card-line">
        <span className="k-chip" title="The model this profile asks for; the agent's own when none">
          {profile.model ?? "agent's default model"}
        </span>
        <span id="phase" className="k-caption">
          {phase}
        </span>
      </div>
      {role ? <div className="k-caption k-muted profile-card-role">{role}</div> : null}
    </div>
  );
}

/// One line about the agent's sign-in, for the rail; the keys themselves are
/// in Setup, where a person goes to change them.
function AuthSummary() {
  const state = useSession();
  const handshake = state.handshakes[state.profileId];
  const vault = state.secrets[state.profileId];
  if (!state.profileId || !handshake) return null;
  let words: string;
  if ("error" in handshake) words = "the agent could not be reached";
  else if (handshake.auth_methods.length === 0) words = "needs no sign-in";
  else {
    const needed = handshake.auth_methods.flatMap((method) => method.vars ?? []).filter((variable) => !variable.optional);
    const present = needed.filter((variable) => vault?.secrets.some((secret) => secret.name === variable.name));
    words = needed.length === 0 ? "signs in on its own" : present.length === needed.length ? "key in place" : "needs a key - see Setup";
  }
  return (
    <div id="auth-summary" className="k-caption k-muted" data-signed-in={words === "key in place" || words === "needs no sign-in" ? "true" : "false"}>
      {words}
    </div>
  );
}

export function SessionRail() {
  const state = useSession();
  const methodId = state.authMethodId;
  // The profile select is the person's own: a value put there by hand (or by
  // a driver) is read when it is acted on, and the store's choice is written
  // back into it when it changes. The route field lives in the dev drawer
  // and is read the same way, by id, because a driver fills it by value.
  const profiles = useRef<HTMLSelectElement>(null);
  useEffect(() => {
    if (profiles.current && profiles.current.value !== state.profileId) profiles.current.value = state.profileId;
  }, [state.profileId, state.profiles]);
  // Open means the host answered with a route; the handle alone is proposed
  // before the open returns, so that a request-scoped elicitation can be
  // polled, and is not yet a session.
  const connected = state.connectionId !== null && state.routeId !== null;
  const configured = state.profiles.length > 0;
  const terminal = state.conversation.terminal;
  const needsAuthentication = Boolean(terminal?.needsAuthentication) || state.problem.includes("requires authentication");
  const open = (mode: "new" | "load" | "resume") => {
    const chosen = profiles.current?.value ?? "";
    if (chosen && chosen !== state.profileId) sessionStore.selectProfile(chosen);
    const route = (document.getElementById("route-id") as HTMLInputElement | null)?.value ?? state.routeInput;
    if (route !== state.routeInput) sessionStore.setRouteInput(route);
    void sessionStore.open(mode, methodId || undefined);
  };
  return (
    <aside className="rail" aria-label="Agent session">
      <div className="brand">
        <span className="brand-mark" />
        SWEM
      </div>
      <div className="k-eyebrow">Agents</div>
      {/* Before a person has an agent this was an empty box under a label,
          which is the first thing they see here and says nothing: not what it
          holds, not that it will fill, not where to start. A control with
          nothing to choose from is not a choice, so it is disabled and the
          line below says where to begin.

          The line is a line and not a placeholder `<option>`: a dozen walks
          read `#profiles option` as "a profile exists" (some as `=== 1`), and
          an option in the empty state would have made every one of them pass
          before there was anything to pass on. */}
      <select
        id="profiles"
        aria-label="Agents"
        ref={profiles}
        disabled={state.profiles.length === 0}
        hidden={state.profiles.length === 0}
        onChange={(event) => sessionStore.selectProfile(event.target.value)}
      >
        {state.profiles.map((profile) => (
          <option value={profile.profile_id} key={profile.profile_id}>
            {profile.profile_id} · {agentName(state.onboarding?.agents, profile.agent_id)}
          </option>
        ))}
      </select>
      {state.profiles.length === 0 ? (
        <p id="no-profiles" className="k-caption k-muted">
          No agent yet. Pick one on the right and it becomes yours.
        </p>
      ) : null}
      <ProfileCard connected={connected} phase={phaseWords(state.phase, connected, state.problem)} />
      <AuthSummary />
      {needsAuthentication ? (
        <div id="auth-needed" className="auth-hint">
          The agent asked to be signed in first. Add what it needs in Setup, then start again.
        </div>
      ) : null}
      <AddProfile />
      <Sessions />
      <div className="session-actions">
        <button id="open-new" className="primary" disabled={!configured || state.busy} onClick={() => open("new")}>
          New session
        </button>
        {/* Load and Resume take a route id from the dev drawer; the session
            list above is how a person comes back to a conversation. They stay
            on the rail, small, for whoever has the id - and for the walks,
            which click them at their real position. */}
        <button id="open-load" className="k-pick" disabled={!configured || state.busy} onClick={() => open("load")}>
          Load
        </button>
        <button id="open-resume" className="k-pick" disabled={!configured || state.busy} onClick={() => open("resume")}>
          Resume
        </button>
      </div>
      <div id="terminal" data-status={terminal?.status ?? ""}>
        {terminal?.summary ?? ""}
      </div>
      <div className="session-actions">
        <button id="cancel" disabled={!connected} onClick={() => sessionStore.cancel()}>
          Stop turn
        </button>
        <button id="disconnect" disabled={!connected} onClick={() => void sessionStore.disconnect()}>
          Disconnect
        </button>
        <button id="close" disabled={!connected} onClick={() => void sessionStore.close()}>
          End session
        </button>
      </div>
      <div id="close-error">{state.problem}</div>
    </aside>
  );
}
