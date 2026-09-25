// First run: the agents this machine already has, offered by name.
//
// An agent that is not installed yet is installed from here, against the
// exact plan the person is shown: the click is the consent, and it consents
// to a named distribution and version, never to a word.

import { sessionStore, useSession } from "./store.ts";

/// What a readiness means to the person reading it, in their words.
///
/// This screen is the first one a person sees, and it showed the enum as the
/// wire spells it: `installed_unverified`, `adapter_required`,
/// `identity_mismatch`. None of those tell a person whether they can use the
/// agent, and the last one is alarming without saying what is wrong. The id
/// stays beside it, because an id is the one thing that is never ambiguous.
function readinessWords(readiness: string): string {
  if (readiness === "handshake_ready") return "ready";
  if (readiness === "installed_unverified") return "on this computer";
  // Found, and the one missing piece named. The previous wording - "on this
  // computer, but SWEM cannot speak to it yet" - was read as "SWEM did not
  // find it", which sent a person looking for a detection bug: the program is
  // on the path and the catalogue says so. What is absent is the bridge the
  // catalogue probes for first, and Install is what fetches it.
  if (readiness === "adapter_required") return "on this computer; Install adds the bridge SWEM talks to it through";
  if (readiness === "identity_mismatch") return "the program found here answers as something else";
  if (readiness === "absent") return "not on this computer";
  return readiness;
}

export function Onboarding() {
  const { onboarding, onboardingStatus } = useSession();
  return (
    <section id="empty-state">
      <h1>Pick an agent to work with.</h1>
      <p>
        These are the coding agents this computer has, or can install. SWEM runs the agent as it is
        and adds what it needs around it: a model, a role, keys, tools, a place to work, and the
        projects you make here.
      </p>
      <div id="agent-options">
        {onboarding?.enabled
          ? onboarding.agents.map((agent) => (
              <div className="agent-option" key={agent.agent_id}>
                <div>
                  <strong>{agent.name}</strong>
                  <small title={`${agent.agent_id} · ${agent.readiness}`}>
                    {agent.agent_id} · {readinessWords(agent.readiness)}
                  </small>
                </div>
                <button
                  className={agent.available ? "create-agent primary" : "create-agent"}
                  data-agent-id={agent.agent_id}
                  onClick={() =>
                    agent.available
                      ? void sessionStore.useAgent(agent.agent_id)
                      : void sessionStore.installAgent(agent.agent_id, (question) => window.confirm(question))
                  }
                >
                  {agent.available ? "Make it mine" : "Install"}
                </button>
              </div>
            ))
          : null}
      </div>
      <div id="onboarding-status">{onboardingStatus}</div>
    </section>
  );
}
