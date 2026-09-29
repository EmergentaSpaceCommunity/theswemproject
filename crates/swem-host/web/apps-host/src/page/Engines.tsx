// Engines: the coding agents an agent can stand on, as this computer has
// them. One is installed from here, by the plan a person is shown.

import { useEffect, useState } from "react";

import type { AgentOption } from "../agent/session.ts";
import { sessionStore, useSession } from "../agent/store.ts";
import { Consent } from "./Consent.tsx";
import { Wrench } from "./icons.tsx";
import { go } from "./place.ts";
import { Standing, UsedBy } from "./standing.tsx";

function standing(engine: AgentOption): { tone: "ready" | "asking" | "none"; says: string } {
  if (engine.available) return { tone: "ready", says: "Installed" };
  if (engine.readiness === "identity_mismatch") return { tone: "asking", says: "Answers as something else" };
  return { tone: "none", says: "Not installed" };
}

export function Engines() {
  const { onboarding, onboardingStatus, profiles } = useSession();
  const [consent, setConsent] = useState<{ asked: string; answer: (yes: boolean) => void } | null>(null);
  useEffect(() => {
    void sessionStore.loadOnboarding();
  }, []);
  const engines = onboarding?.enabled ? onboarding.agents : [];
  return (
    <>
      <Consent
        asked={consent?.asked ?? null}
        onAnswer={(yes) => {
          consent?.answer(yes);
          setConsent(null);
        }}
      />
      <section className="k-card k-stack">
        <div className="k-spread">
          <div className="w-col w-close">
            <h2 className="k-heading">Engines</h2>
            <span className="k-caption">The coding agents an agent can stand on. SWEM runs one as it is and adds what it needs around it.</span>
          </div>
          <button type="button" className="k-btn" onClick={() => go({ at: "store" })}>
            Browse the registry
          </button>
        </div>
        {onboardingStatus ? <div className="k-notice">{onboardingStatus}</div> : null}
        {engines.length === 0 ? (
          <span className="k-caption">Looking at what this computer has…</span>
        ) : (
          <table className="w-table">
            <thead>
              <tr>
                <th scope="col" className="k-eyebrow">Engine</th>
                <th scope="col" className="k-eyebrow">Agents</th>
                <th scope="col" className="k-eyebrow w-end">State</th>
              </tr>
            </thead>
            <tbody>
              {engines.map((engine) => {
                const how = standing(engine);
                return (
                  <tr key={engine.agent_id}>
                    <td>
                      <span className="k-inline w-nowrap">
                        <Wrench size={18} />
                        <span className="k-name">{engine.name}</span>
                      </span>
                    </td>
                    <td>
                      <UsedBy profiles={profiles.filter((profile) => profile.agent_id === engine.agent_id).map((profile) => profile.profile_id)} />
                    </td>
                    <td className="w-end">
                      <span className="k-inline w-tight w-end">
                        <Standing tone={how.tone}>{how.says}</Standing>
                        {engine.available ? null : (
                          <button
                            type="button"
                            className="k-btn"
                            onClick={() => void sessionStore.installAgent(engine.agent_id, (asked) => new Promise<boolean>((answer) => setConsent({ asked, answer })), false)}
                          >
                            Install
                          </button>
                        )}
                      </span>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </section>
    </>
  );
}
