// A new agent, in four short steps: what it is built on and who it is;
// where it works, its model and who keeps its time; how it signs in and
// what it will have where it lives; what it may do.
//
// What a step sets is what Settings sets, by the same operations. The
// agent is made when the second step is left; the two after it change an
// agent that is there, and can be skipped.

import { useEffect, useState, type ReactNode } from "react";
import { useStore } from "zustand";

import type { AgentOption } from "../agent/session.ts";
import { sessionStore, useSession } from "../agent/store.ts";
import { fetchJson } from "../http.ts";
import { Consent } from "./Consent.tsx";
import { sized } from "./files.ts";
import { Box, Check, Laptop } from "./icons.tsx";
import { looks, programsFound, took, type LookInside } from "./looks.ts";
import { go } from "./place.ts";
import { providers as providersStore, providing, when } from "./providers.ts";
import { SignIn } from "./settings/SignIn.tsx";
import { Standing } from "./standing.tsx";
import { KEEPER_NAMES, time, timing } from "./time.ts";
import { act, world } from "./world.ts";

const STEPS = ["Engine and identity", "Put together", "Sign-in and a look inside", "What it may do"];
const COLOURS = ["info", "success", "warning", "danger"];

/// What a readiness means to the person reading it.
function readiness(engine: AgentOption): string {
  if (engine.available) return "Installed";
  if (engine.readiness === "identity_mismatch") return "The program found here answers as something else";
  return "Not installed";
}

const named = (name: string): string =>
  name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48);

const send = (method: string, body: unknown): RequestInit => ({ method, headers: { "content-type": "application/json" }, body: JSON.stringify(body) });

function Steps({ at }: { at: number }) {
  return (
    <ol className="w-steps-across" aria-label="Steps">
      {STEPS.map((step, index) => (
        <li className={`w-step-across${index === at ? " k-active" : ""}${index < at ? " w-done" : ""}`} aria-current={index === at ? "step" : undefined} key={step}>
          <span className="w-step-number">{index < at ? <Check size={13} /> : index + 1}</span>
          <span>{step}</span>
        </li>
      ))}
    </ol>
  );
}

function Part({ title, about, action, children }: { title: string; about?: string; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="k-card k-stack w-part" aria-label={title}>
      <div className="k-spread">
        <div className="w-col w-close">
          <h2 className="k-heading">{title}</h2>
          {about ? <span className="k-caption">{about}</span> : null}
        </div>
        {action}
      </div>
      {children}
    </section>
  );
}

function Row({ name, children, tone }: { name: string; children: ReactNode; tone?: "ready" | "asking" | "none" }) {
  return (
    <div className="k-row w-nowrap">
      <span className={`k-dot k-small${tone && tone !== "none" ? ` k-${tone}` : ""}`} />
      <span className="k-name k-grow">{name}</span>
      <span className="k-caption w-end-text">{children}</span>
    </div>
  );
}

/// What the agent will have where it lives, as it was found from inside.
function Inside({ profile, name, engineName }: { profile: string; name: string; engineName: string }) {
  const [look, setLook] = useState<LookInside | null>(null);
  const [looking, setLooking] = useState(false);
  const [problem, setProblem] = useState("");
  const again = async () => {
    setLooking(true);
    setProblem("");
    try {
      setLook(await looks.now(profile));
    } catch (error) {
      setProblem((error as Error).message);
    } finally {
      setLooking(false);
    }
  };
  useEffect(() => {
    void again();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profile]);
  const engine = look?.engine;
  const machine = look?.machine;
  const tools = machine ? programsFound(machine.programs, ["git", "node", "python3"]) : null;
  return (
    <Part
      title={`What ${name} will have there`}
      about={look ? `Looked at from inside its machine at ${when(look.looked_ms)}.` : looking ? "Being looked at from inside its machine…" : ""}
      action={
        <button type="button" className="k-btn k-quiet" disabled={looking} onClick={() => void again()}>
          {looking ? "Looking…" : "Look again"}
        </button>
      }
    >
      {problem ? <div className="k-notice k-danger">{problem}</div> : null}
      {engine ? (
        <div className="k-stack w-close">
          <Row name={engineName || engine.name || "Its engine"} tone={engine.starts ? "ready" : "asking"}>
            {engine.starts ? [engine.version, engine.started_in_ms != null ? `starts in ${took(engine.started_in_ms)}` : ""].filter(Boolean).join(" · ") : "Does not start"}
          </Row>
          {engine.starts ? (
            <Row name={engine.signed_in === false ? "Not signed in to its model" : engine.signed_in ? "Signed in to its model" : "Its sign-in"} tone={engine.signed_in ? "ready" : "asking"}>
              {engine.signed_in ? (engine.answered_in_ms != null ? `answered in ${took(engine.answered_in_ms)}` : "") : engine.signed_in === false ? "Sign it in above" : "Could not be told"}
            </Row>
          ) : null}
          {!engine.starts || (engine.signed_in !== true && engine.said) ? <pre className="k-mono w-said">{engine.said}</pre> : null}
          {machine ? (
            <>
              <Row name={tools?.names || "git, node, python"} tone={tools?.names ? "ready" : "none"}>
                {tools?.versions || "None of them is there"}
              </Row>
              <Row name="Its workspace" tone={machine.workspace_writable ? "ready" : "asking"}>
                {[
                  machine.workspace_free_bytes != null ? `${sized(machine.workspace_free_bytes)} free` : "",
                  machine.workspace_writable ? (machine.workspace_owned === false ? "somebody else owns the files" : "you own the files") : "it cannot write there",
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </Row>
            </>
          ) : (
            <Row name="Its machine" tone="none">
              Looked at from inside once it keeps a machine of its own
            </Row>
          )}
          <Row name="Containers" tone={look.containers ? "ready" : "none"}>
            {look.containers ? "Can be started there" : (look.containers_said || "None can be started there").replace(/^./, (first) => first.toUpperCase()).replace(/\.$/, "")}
          </Row>
          {look.containers ? null : (
            <span className="k-caption">Work that needs a container of its own, such as checks against a real database, will say it is blocked here.</span>
          )}
        </div>
      ) : null}
    </Part>
  );
}

export function NewAgent() {
  const { onboarding, onboardingStatus, profiles, environments, permissionProfiles } = useSession();
  const participants = useStore(world, (state) => state.participants);
  const models = useStore(providersStore, (state) => state.models);
  const standing = useStore(providersStore, (state) => state.environments);
  const keeper = useStore(time, (known) => known.keeper);
  const [at, setAt] = useState(0);
  const [engine, setEngine] = useState("");
  const [name, setName] = useState("");
  const [handle, setHandle] = useState("");
  const [colour, setColour] = useState(COLOURS[0] ?? "info");
  const [role, setRole] = useState("");
  const [place, setPlace] = useState("");
  const [provider, setProvider] = useState("");
  const [model, setModel] = useState("");
  const [kept, setKept] = useState("");
  const [permission, setPermission] = useState("");
  const [made, setMade] = useState("");
  const [making, setMaking] = useState(false);
  const [problem, setProblem] = useState("");
  const [consent, setConsent] = useState<{ asked: string; answer: (yes: boolean) => void } | null>(null);

  useEffect(() => {
    void sessionStore.loadOnboarding();
    void sessionStore.loadEnvironments();
    void sessionStore.loadPermissionProfiles();
    void providing.models();
    void providing.environments();
    void timing.keeper().catch(() => {});
  }, []);

  const engines = onboarding?.enabled ? onboarding.agents : [];
  const chosen = engines.find((one) => one.agent_id === engine) ?? null;
  const id = named(name);
  const taken = made === "" && profiles.some((profile) => profile.profile_id === id);
  const called = (handle.trim().replace(/^@/, "") || id).toLowerCase();
  const agent = Object.values(participants).find((one) => one.profile_id === made);
  const places = environments.filter((one) => one.environment_profile_id);
  const chosenPlace = places.find((one) => one.environment_profile_id === place) ?? places.find((one) => one.available !== false) ?? null;
  const from = (models ?? []).find((one) => one.id === provider) ?? null;
  const firstStepDone = Boolean(chosen?.available && id && !taken);

  const summary = [name.trim(), id ? `@${called}` : "", chosen?.name, at > 0 ? chosenPlace?.name : "", at > 1 && model ? from?.models.find((one) => one.id === model)?.name || model : ""].filter(Boolean).join(" · ");

  /// Make the agent of what the first two steps hold.
  const make = async (): Promise<boolean> => {
    if (made) return true;
    if (!chosen || !id) return false;
    setMaking(true);
    setProblem("");
    try {
      const setup = provider || model || role.trim() ? { setup: { model_provider: provider || null, model: model || null, role, agent_skills: [] } } : {};
      await fetchJson("/api/profiles/local", send("POST", { agent_id: chosen.agent_id, profile_id: id, ...setup }));
      setMade(id);
      await sessionStore.loadProfiles(id);
      await act.people();
      const who = Object.values(world.getState().participants).find((one) => one.profile_id === id);
      if (who) {
        await fetchJson(`/api/people/${encodeURIComponent(who.participant_id)}`, send("PATCH", { name: name.trim(), handle: called, colour }));
        await act.people();
      }
      const lives = chosenPlace?.environment_profile_id;
      const has = sessionStore.getSnapshot().profiles.find((one) => one.profile_id === id)?.environment_profile_id;
      if (lives && lives !== has && !(await sessionStore.amendProfile({ environment: lives }))) throw new Error(sessionStore.getSnapshot().environmentStatus || "Where it works could not be set.");
      if (kept) await timing.choose(id, kept);
      return true;
    } catch (error) {
      setProblem((error as Error).message);
      return false;
    } finally {
      setMaking(false);
    }
  };

  const open = () => {
    const who = Object.values(world.getState().participants).find((one) => one.profile_id === (made || id));
    if (who) go({ at: "agent", agent: who.participant_id, tab: "chat" });
  };

  const install = (one: AgentOption) =>
    void sessionStore
      .installAgent(one.agent_id, (asked) => new Promise<boolean>((answer) => setConsent({ asked, answer })), false)
      .then(() => setEngine(one.agent_id));

  return (
    <main className="w-main w-scroll">
      <Consent
        asked={consent?.asked ?? null}
        onAnswer={(yes) => {
          consent?.answer(yes);
          setConsent(null);
        }}
      />
      <form
        className="w-form w-wizard"
        onSubmit={(event) => {
          event.preventDefault();
        }}
      >
        <div className="k-stack w-close">
          <h1 className="w-h1">New agent</h1>
          <p className="k-caption">{at === 0 ? "Four short steps. Everything can be changed later." : summary}</p>
        </div>
        <Steps at={at} />

        {at === 0 ? (
          <>
            <Part title="What it is built on" about="An agent here stands on a coding agent you already trust. SWEM gives it a name, an environment, keys, chats and a sense of time.">
              <fieldset className="w-fieldset w-cards">
                {engines.map((one) => (
                  <label className={`k-rail-item${engine === one.agent_id ? " k-active" : ""}`} key={one.agent_id}>
                    <input type="radio" name="engine" checked={engine === one.agent_id} onChange={() => setEngine(one.agent_id)} />
                    <span className="k-two">
                      <span className="k-name">{one.name}</span>
                      <span className="k-caption">{readiness(one)}</span>
                    </span>
                    {one.available ? null : (
                      <button type="button" className="k-btn" onClick={() => install(one)}>
                        Install
                      </button>
                    )}
                  </label>
                ))}
                <div className="k-rail-item">
                  <span className="k-two">
                    <span className="k-name">Another agent</span>
                    <span className="k-caption">Any agent from the public registry.</span>
                  </span>
                  <button type="button" className="k-btn" onClick={() => go({ at: "store" })}>
                    Browse
                  </button>
                </div>
              </fieldset>
              {engines.length === 0 ? <div className="k-caption">Looking at what this computer has…</div> : null}
              {onboardingStatus ? <div className="k-notice">{onboardingStatus}</div> : null}
            </Part>
            <Part title="Who it is">
              <div className="w-pair">
                <label className="k-stack w-close">
                  <span className="k-caption">Name</span>
                  <input className="k-field" value={name} maxLength={64} placeholder="Reviewer" autoFocus onChange={(event) => setName(event.target.value)} />
                  {id && taken ? <span className="k-caption k-is-danger">There is an agent called {id} already.</span> : null}
                </label>
                <label className="k-stack w-close">
                  <span className="k-caption">Handle</span>
                  <input className="k-field k-mono" value={handle} maxLength={32} spellCheck={false} placeholder={id ? `@${id}` : "@reviewer"} onChange={(event) => setHandle(event.target.value)} />
                  <span className="k-caption">What it is named by in a chat.</span>
                </label>
              </div>
              <div className="k-stack w-close">
                <span className="k-caption">Colour</span>
                <div className="k-inline w-tight" role="radiogroup" aria-label="Colour">
                  {COLOURS.map((one) => (
                    <button
                      type="button"
                      role="radio"
                      aria-checked={colour === one}
                      aria-label={one}
                      className={`k-avatar k-is-${one} w-swatch${colour === one ? " k-active" : ""}`}
                      key={one}
                      onClick={() => setColour(one)}
                    >
                      {(name.trim().slice(0, 1) || "A").toUpperCase()}
                    </button>
                  ))}
                </div>
              </div>
              <label className="k-stack w-close">
                <span className="k-caption">What it is for</span>
                <textarea className="k-field" rows={4} value={role} placeholder="Reads what was changed and says what is wrong with it, shortly." onChange={(event) => setRole(event.target.value)} />
                <span className="k-caption">Written in your words. It becomes the agent's standing instructions.</span>
              </label>
            </Part>
          </>
        ) : null}

        {at === 1 ? (
          <>
            <Part
              title="Where it runs"
              about={`How this host runs ${name.trim() || "it"}: as is on this machine, or sealed in a container. Its files, its terminal, its sign-in and whatever it starts stay on this host.`}
              action={
                <button type="button" className="k-btn k-quiet" onClick={() => go({ at: "providers", tab: "environments" })}>
                  Environments
                </button>
              }
            >
              <fieldset className="w-fieldset" disabled={made !== ""}>
                {places.map((one) => {
                  const inAContainer = one.environment_profile_id.includes("container");
                  const known = (standing ?? []).find((known) => known.environment_profile_id === one.environment_profile_id);
                  const can = one.available !== false;
                  return (
                    <label className={`k-rail-item w-top${chosenPlace?.environment_profile_id === one.environment_profile_id ? " k-active" : ""}`} key={one.environment_profile_id}>
                      <input type="radio" name="place" disabled={!can} checked={chosenPlace?.environment_profile_id === one.environment_profile_id} onChange={() => setPlace(one.environment_profile_id)} />
                      {inAContainer ? <Box size={18} /> : <Laptop size={18} />}
                      <span className="w-col w-close k-grow">
                        <span className="k-name">{known?.name ?? one.name}</span>
                        <span className="k-caption">{one.summary}</span>
                        {known ? (
                          <span className="k-inline w-tight">
                            {known.an_agent_gets.split(", ").map((gets) => (
                              <span className="k-badge" key={gets}>
                                {gets.replace(/^./, (first) => first.toUpperCase())}
                              </span>
                            ))}
                          </span>
                        ) : null}
                        {can ? null : <span className="k-caption k-is-warning">{one.why_not}</span>}
                      </span>
                      <Standing tone={can ? "ready" : "asking"}>{can ? "Ready" : "Cannot start"}</Standing>
                    </label>
                  );
                })}
              </fieldset>
              {made ? <span className="k-caption">It is made. Where it works is changed in its Settings.</span> : null}
            </Part>
            <Part
              title="Its model"
              about="From the models you have set up."
              action={
                <button type="button" className="k-btn k-quiet" onClick={() => go({ at: "providers", tab: "models" })}>
                  Add
                </button>
              }
            >
              <div className="w-pair">
                <label className="k-stack w-close">
                  <span className="k-caption">Provider</span>
                  <select
                    className="k-field"
                    value={provider}
                    disabled={made !== ""}
                    onChange={(event) => {
                      setProvider(event.target.value);
                      setModel("");
                    }}
                  >
                    <option value="">Its engine's own</option>
                    {(models ?? []).map((one) => (
                      <option value={one.id} key={one.id}>
                        {one.name}
                        {one.key ? "" : " · no key given"}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="k-stack w-close">
                  <span className="k-caption">Model</span>
                  <select className="k-field" value={model} disabled={made !== "" || !from || from.models.length === 0} onChange={(event) => setModel(event.target.value)}>
                    <option value="">Its engine's own choice</option>
                    {(from?.models ?? []).map((one) => (
                      <option value={one.id} key={one.id}>
                        {one.name || one.id}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
            </Part>
            <Part title="Who keeps time for it" about="For schedules and reminders.">
              <label className="k-stack w-close">
                <span className="k-caption">Timekeeper</span>
                <select className="k-field" value={kept} disabled={made !== ""} onChange={(event) => setKept(event.target.value)}>
                  <option value="">{`${KEEPER_NAMES[keeper?.keepers.find((one) => one.default)?.id ?? "swem"]} · default`}</option>
                  {(keeper?.keepers ?? [])
                    .filter((one) => one.id !== "outside" && !one.default)
                    .map((one) => (
                      <option value={one.id} key={one.id} disabled={one.id === "system" && !one.on}>
                        {one.id === "system" && !one.on ? `${KEEPER_NAMES[one.id]} (off)` : KEEPER_NAMES[one.id]}
                      </option>
                    ))}
                </select>
              </label>
            </Part>
          </>
        ) : null}

        {at === 2 && made ? (
          <>
            <Part title="Sign-in" about={`Done where ${name.trim()} lives, because that is where it will use it.`}>
              <SignIn needsSignIn={false} />
            </Part>
            <Inside profile={made} name={name.trim()} engineName={chosen?.name ?? ""} />
          </>
        ) : null}

        {at === 3 && made ? (
          <Part title="What it may do" about={`How ${name.trim()} asks before it changes something or runs a command.`}>
            <fieldset className="w-fieldset">
              {permissionProfiles.map((one) => {
                const has = permission || sessionStore.getSnapshot().profiles.find((known) => known.profile_id === made)?.permission_profile_id || "";
                return (
                  <label className={`k-rail-item w-top${has === one.permission_profile_id ? " k-active" : ""}`} key={one.permission_profile_id}>
                    <input
                      type="radio"
                      name="permission"
                      checked={has === one.permission_profile_id}
                      onChange={() => {
                        setPermission(one.permission_profile_id);
                        void sessionStore.amendProfile({ permissions: one.permission_profile_id });
                      }}
                    />
                    <span className="w-col w-close">
                      <span className="k-name">{one.name}</span>
                      <span className="k-caption">{one.summary}</span>
                    </span>
                  </label>
                );
              })}
            </fieldset>
            <span className="k-caption">Its tools and skills are given in its Settings.</span>
          </Part>
        ) : null}

        {problem ? <div className="k-notice k-danger">{problem}</div> : null}
        <div className="k-spread">
          <span>
            {at === 1 && !made ? (
              <button type="button" className="k-btn k-quiet" onClick={() => setAt(0)}>
                Back
              </button>
            ) : at === 3 ? (
              <button type="button" className="k-btn k-quiet" onClick={() => setAt(2)}>
                Back
              </button>
            ) : at === 0 ? (
              <button type="button" className="k-btn k-quiet" onClick={() => history.back()}>
                Cancel
              </button>
            ) : null}
          </span>
          <span className="k-inline w-tight">
            {at === 1 || at === 2 ? (
              <button
                type="button"
                className="k-btn"
                disabled={making}
                onClick={() => {
                  void make().then((there) => (there ? open() : undefined));
                }}
              >
                {made ? "Finish now" : "Skip the rest and create"}
              </button>
            ) : null}
            {at === 0 ? (
              <button type="button" className="k-btn k-primary" disabled={!firstStepDone} onClick={() => setAt(1)}>
                Continue
              </button>
            ) : at === 1 ? (
              <button
                type="button"
                className="k-btn k-primary"
                disabled={making}
                onClick={() => {
                  void make().then((there) => (there ? setAt(2) : undefined));
                }}
              >
                {making ? "Making it…" : "Continue"}
              </button>
            ) : at === 2 ? (
              <button type="button" className="k-btn k-primary" onClick={() => setAt(3)}>
                Continue
              </button>
            ) : (
              <button type="button" className="k-btn k-primary" disabled={!agent} onClick={open}>
                Open its chat
              </button>
            )}
          </span>
        </div>
      </form>
    </main>
  );
}
