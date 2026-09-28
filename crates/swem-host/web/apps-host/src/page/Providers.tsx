// Providers: what agents are put together from. Set up once, with its key,
// then used by any number of agents.

import { Dialog } from "@base-ui/react/dialog";
import { useEffect, useState } from "react";
import { useStore } from "zustand";

import { Box, Brain, Laptop } from "./icons.tsx";
import { go, type ProvidersTab } from "./place.ts";
import { gigabytes, providers, providing, when, type EngineLook, type HostStanding, type ModelProviderStanding } from "./providers.ts";
import { Avatar } from "./who.tsx";
import { world } from "./world.ts";

const TABS: { tab: ProvidersTab; label: string; sign: typeof Brain }[] = [
  { tab: "models", label: "Models", sign: Brain },
  { tab: "hosts", label: "Hosts", sign: Laptop },
];

/// The agents that stand on something, by their faces.
function UsedBy({ profiles }: { profiles: string[] }) {
  const participants = useStore(world, (state) => state.participants);
  const agents = Object.values(participants).filter((one) => one.kind === "agent" && one.profile_id && profiles.includes(one.profile_id));
  if (agents.length === 0) return <span className="k-caption">No agents yet</span>;
  return (
    <span className="k-inline w-tight w-nowrap" title={agents.map((agent) => agent.name).join(", ")}>
      {agents.map((agent) => (
        <Avatar who={agent} size="small" key={agent.participant_id} />
      ))}
    </span>
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

/// Giving a key: typed once, sent, and gone from the page.
function GiveKey({ provider, onClose }: { provider: ModelProviderStanding | null; onClose: () => void }) {
  const [value, setValue] = useState("");
  const [variable, setVariable] = useState("");
  const [problem, setProblem] = useState("");
  const [giving, setGiving] = useState(false);
  const keptBy = useStore(providers, (state) => state.keptBy);
  const close = () => {
    setValue("");
    setVariable("");
    setProblem("");
    onClose();
  };
  const named = Boolean(provider?.key_env);
  const give = async () => {
    if (!provider) return;
    setGiving(true);
    setProblem("");
    try {
      await providing.giveKey(provider.id, value, named ? undefined : variable.trim());
      close();
    } catch (error) {
      setProblem((error as Error).message);
    } finally {
      setGiving(false);
    }
  };
  return (
    <Dialog.Root open={provider !== null} onOpenChange={(next) => (next ? undefined : close())}>
      <Dialog.Portal>
        <Dialog.Backdrop className="w-scrim" />
        <Dialog.Popup className="k-dialog w-dialog">
          <Dialog.Title className="k-heading">
            {provider?.name}: {provider?.key_label}
          </Dialog.Title>
          <Dialog.Description className="k-caption">
            Given once, for every agent that answers from {provider?.name}. {keptBy ? `It is kept in ${keptBy}.` : ""} It is never shown again.
          </Dialog.Description>
          <form
            className="k-stack"
            onSubmit={(event) => {
              event.preventDefault();
              void give();
            }}
          >
            <label className="k-stack w-close">
              <span className="k-caption">{provider?.key ? "The new one" : "What you were given"}</span>
              <input className="k-field" type="password" autoComplete="off" spellCheck={false} value={value} onChange={(event) => setValue(event.target.value)} autoFocus />
            </label>
            {named ? null : (
              <label className="k-stack w-close">
                <span className="k-caption">The variable the agent reads it from</span>
                <input className="k-field k-mono" value={variable} spellCheck={false} onChange={(event) => setVariable(event.target.value)} placeholder="DESK_API_KEY" />
              </label>
            )}
            {problem ? <div className="k-notice k-danger">{problem}</div> : null}
            <div className="k-inline w-end">
              <Dialog.Close className="k-btn k-quiet" type="button">
                Not now
              </Dialog.Close>
              <button type="submit" className="k-btn k-primary" disabled={giving || value.trim() === "" || (!named && variable.trim() === "")}>
                {provider?.key ? "Replace it" : "Give it"}
              </button>
            </div>
          </form>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function Models() {
  const models = useStore(providers, (state) => state.models);
  const [giving, setGiving] = useState<string | null>(null);
  const [problem, setProblem] = useState("");
  useEffect(() => {
    void providing.models();
  }, []);
  if (models === null) return <div className="k-caption">Reading the providers…</div>;
  const take = (provider: ModelProviderStanding) => {
    setProblem("");
    providing.takeKey(provider.id).catch((error: Error) => setProblem(error.message));
  };
  return (
    <section className="k-card k-stack w-scroll">
      <table className="w-table">
        <thead>
          <tr>
            <th scope="col" className="k-eyebrow">Provider</th>
            <th scope="col" className="k-eyebrow">What opens it</th>
            <th scope="col" className="k-eyebrow">Models</th>
            <th scope="col" className="k-eyebrow w-end">Agents</th>
          </tr>
        </thead>
        <tbody>
          {models.map((provider) => (
            <tr key={provider.id}>
              <td>
                <span className="k-two">
                  <span className="k-name">{provider.name}</span>
                  <span className="k-caption">{provider.base_url || (provider.origin === "built_in" ? "Built in" : "Added by you")}</span>
                </span>
              </td>
              <td>
                <span className="k-inline w-tight w-nowrap">
                  <span className="k-two">
                    <span>{provider.key_label}</span>
                    {provider.key ? (
                      <Standing tone="ready">{`Given ${when(provider.key.given_ms)}`}</Standing>
                    ) : (
                      <span className="k-caption">Not given here</span>
                    )}
                  </span>
                  <button type="button" className="k-btn k-quiet" onClick={() => setGiving(provider.id)}>
                    {provider.key ? "Replace" : "Give"}
                  </button>
                  {provider.key ? (
                    <button type="button" className="k-btn k-quiet" onClick={() => take(provider)}>
                      Take away
                    </button>
                  ) : null}
                </span>
              </td>
              <td>{provider.models.length > 0 ? provider.models.map((model) => model.name).join(", ") : <span className="k-caption">Any it serves</span>}</td>
              <td className="w-end">
                <UsedBy profiles={provider.used_by} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <span className="k-caption">
        An agent is handed what is given here when it starts. Without it, an agent signs in the way its engine does.
      </span>
      {problem ? <div className="k-notice k-danger">{problem}</div> : null}
      <GiveKey provider={models.find((one) => one.id === giving) ?? null} onClose={() => setGiving(null)} />
    </section>
  );
}

function Engine({ name, found }: { name: string; found: EngineLook }) {
  const tone = found.standing === "ready" ? "ready" : found.standing === "not_ready" ? "asking" : "none";
  const words = found.standing === "not_ready" ? found.said.replace(/^./, (first) => first.toUpperCase()) : found.said;
  return (
    <div className="w-fact">
      <span className="k-caption">{name}</span>
      <Standing tone={tone}>{found.standing === "ready" && found.version ? `Ready · ${found.version}` : words}</Standing>
    </div>
  );
}

function Fact({ name, children }: { name: string; children: string }) {
  return (
    <div className="w-fact">
      <span className="k-caption">{name}</span>
      <span className="k-name">{children}</span>
    </div>
  );
}

function Host({ host }: { host: HostStanding }) {
  return (
    <tr>
      <td>
        <span className="k-inline w-nowrap">
          {host.kind === "Built in" ? <Laptop size={18} /> : <Box size={18} />}
          <span className="k-two">
            <span className="k-name">{host.name}</span>
            <span className="k-caption">{host.kind}</span>
          </span>
        </span>
      </td>
      <td>
        <span className="k-caption">None needed</span>
      </td>
      <td>{host.machine || <span className="k-caption">Not known</span>}</td>
      <td>{host.an_agent_gets}</td>
      <td>
        <UsedBy profiles={host.used_by} />
      </td>
      <td className="w-end">
        <Standing tone={host.ready ? "ready" : "asking"}>{host.ready ? "Ready" : "Cannot start"}</Standing>
        {host.ready ? null : <div className="k-caption">{host.said}</div>}
      </td>
    </tr>
  );
}

function Hosts() {
  const machine = useStore(providers, (state) => state.machine);
  const hosts = useStore(providers, (state) => state.hosts);
  const looking = useStore(providers, (state) => state.looking);
  useEffect(() => {
    void providing.hosts();
  }, []);
  return (
    <>
      <section className="k-card k-stack">
        <div className="k-spread">
          <div className="k-inline w-nowrap">
            <Laptop size={20} />
            <div className="w-col w-close">
              <span className="k-title">The machine SWEM runs on</span>
              <span className="k-caption">{machine ? `Looked at ${when(machine.looked_ms)}.` : looking ? "Being looked at…" : "Not looked at yet."}</span>
            </div>
          </div>
          <button type="button" className="k-btn k-quiet" disabled={looking} onClick={() => void providing.lookAgain()}>
            {looking ? "Looking…" : "Look again"}
          </button>
        </div>
        {machine ? (
          <div className="w-facts">
            <Fact name="System">{`${machine.system} · ${machine.architecture}`}</Fact>
            <Fact name="Processor">{`${machine.processors} cores`}</Fact>
            <Fact name="Memory">{`${gigabytes(machine.memory_bytes)} · ${gigabytes(machine.memory_free_bytes)} free`}</Fact>
            <Fact name="Disk">{machine.disk_free_bytes == null ? "Not known" : `${gigabytes(machine.disk_free_bytes)} free`}</Fact>
            <Engine name="Podman" found={machine.podman} />
            <Engine name="Docker" found={machine.docker} />
          </div>
        ) : null}
      </section>
      <section className="k-card w-scroll">
        <table className="w-table">
          <thead>
            <tr>
              <th scope="col" className="k-eyebrow">Host</th>
              <th scope="col" className="k-eyebrow">Key</th>
              <th scope="col" className="k-eyebrow">Machine</th>
              <th scope="col" className="k-eyebrow">An agent gets</th>
              <th scope="col" className="k-eyebrow">Agents</th>
              <th scope="col" className="k-eyebrow w-end">State</th>
            </tr>
          </thead>
          <tbody>{(hosts ?? []).map((host) => <Host host={host} key={host.id} />)}</tbody>
        </table>
      </section>
    </>
  );
}

export function Providers({ tab }: { tab: ProvidersTab }) {
  const problem = useStore(providers, (state) => state.problem);
  return (
    <main className="w-main">
      <header className="w-head">
        <div className="w-col w-close">
          <h1 className="w-h1">Providers</h1>
          <span className="k-caption">What agents are put together from. Set up once, with its key, then used by any number of agents.</span>
        </div>
        <nav className="k-tabs" aria-label="Providers">
          {TABS.map((one) => (
            <button
              type="button"
              className={`k-tab${tab === one.tab ? " k-active" : ""}`}
              aria-current={tab === one.tab ? "page" : undefined}
              key={one.tab}
              onClick={() => go({ at: "providers", tab: one.tab })}
            >
              <one.sign size={15} />
              <span>{one.label}</span>
            </button>
          ))}
        </nav>
      </header>
      <div className="w-page">
        {problem ? <div className="k-notice k-danger">{problem}</div> : null}
        {tab === "hosts" ? <Hosts /> : <Models />}
      </div>
    </main>
  );
}
