// A new agent: what it is called and what it stands on.
//
// Any number of agents stand on one engine; each has a name of its own, a
// place to work of its own and a home of its own, so two are never one.

import { useEffect, useState } from "react";
import { useStore } from "zustand";

import { sessionStore, useSession } from "../agent/store.ts";
import { go } from "./place.ts";
import { act, world } from "./world.ts";

/// What a readiness means to the person reading it.
function readiness(said: string): string {
  if (said === "handshake_ready") return "Ready";
  if (said === "installed_unverified") return "On this computer";
  if (said === "adapter_required") return "On this computer; installing adds the bridge SWEM talks to it through";
  if (said === "identity_mismatch") return "The program found here answers as something else";
  if (said === "absent") return "Not on this computer";
  return said;
}

const named = (name: string): string =>
  name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48);

export function NewAgent() {
  const { onboarding, onboardingStatus, profiles } = useSession();
  const participants = useStore(world, (state) => state.participants);
  const [name, setName] = useState("");
  const [engine, setEngine] = useState("");
  const [before] = useState(() => new Set(profiles.map((profile) => profile.profile_id)));
  const engines = onboarding?.enabled ? onboarding.agents : [];
  const chosen = engines.find((one) => one.agent_id === engine) ?? null;
  const id = named(name);
  const taken = profiles.some((profile) => profile.profile_id === id);
  // Made: the product goes to it as soon as the host says who it is.
  useEffect(() => {
    const made = profiles.find((profile) => !before.has(profile.profile_id));
    if (!made) return;
    const agent = Object.values(participants).find((one) => one.profile_id === made.profile_id);
    if (agent) go({ at: "agent", agent: agent.participant_id, tab: "chat" });
    else void act.people();
  }, [profiles, participants, before]);
  return (
    <main className="w-main w-scroll">
      <form
        className="w-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (chosen?.available && id && !taken) void sessionStore.useAgent(chosen.agent_id, id);
        }}
      >
        <div className="k-stack w-close">
          <h1 className="w-h1">New agent</h1>
          <p className="k-caption">
            An agent stands on a coding agent this computer has. SWEM runs it as it is and adds what it needs around it: a
            model, a role, keys, tools and a place to work.
          </p>
        </div>
        <label className="k-stack w-close">
          <span className="k-caption">What it is called</span>
          <input className="k-field" value={name} maxLength={64} placeholder="Reviewer" autoFocus onChange={(event) => setName(event.target.value)} />
          {id ? <span className={`k-caption${taken ? " k-is-danger" : ""}`}>{taken ? `There is an agent called ${id} already.` : `It is named in a chat as @${id}.`}</span> : null}
        </label>
        <fieldset className="w-fieldset">
          <legend className="k-caption">What it stands on</legend>
          {engines.map((one) => (
            <label className={`k-rail-item${engine === one.agent_id ? " k-active" : ""}`} key={one.agent_id}>
              <input type="radio" name="engine" checked={engine === one.agent_id} onChange={() => setEngine(one.agent_id)} />
              <span className="k-two">
                <span className="k-name">{one.name}</span>
                <span className="k-caption">{readiness(one.readiness)}</span>
              </span>
              {one.available ? null : (
                <button type="button" className="k-btn" onClick={() => void sessionStore.installAgent(one.agent_id, (question) => window.confirm(question))}>
                  Install
                </button>
              )}
            </label>
          ))}
          {engines.length === 0 ? <div className="k-caption">Looking at what this computer has…</div> : null}
        </fieldset>
        {onboardingStatus ? <div className="k-notice">{onboardingStatus}</div> : null}
        <div className="k-inline w-end">
          <button type="submit" className="k-btn k-primary" disabled={!chosen?.available || !id || taken}>
            Make the agent
          </button>
        </div>
      </form>
    </main>
  );
}
