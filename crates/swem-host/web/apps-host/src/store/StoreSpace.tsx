// The Store: one list of what a person can install, from the indexes this
// product reads - the ACP registry's agents, and the catalogs of MCP servers
// and skills a person adds by URL.
//
// Installing here is the same act as everywhere in the product: the exact
// plan is read, the person consents to it by name in a dialog, and the
// install is applied against that plan's id. What differs is where the thing
// lands afterwards: an agent is offered in the Agent space, a server is
// declared for a profile to attach, a skill is there for a profile to take.
import { useEffect, useState } from "react";

import { sessionStore } from "../agent/store.ts";
import { fetchJson } from "../project/api";

type Kind = "agent" | "server" | "skill" | "tool";

interface StoreEntry {
  kind: Kind;
  id: string;
  name: string;
  description: string;
  version: string;
  index: string;
  installable: boolean;
  reason?: string;
  needs?: string[];
  installed?: string;
  bundled?: boolean;
}

interface StoreIndex {
  slug: string;
  name: string;
  url: string;
  entries: number;
  builtin?: boolean;
}

interface StoreView {
  registry: { url: string; agents: number; read: string; error?: string };
  indexes: StoreIndex[];
  entries: StoreEntry[];
}

interface InstallPlan {
  plan_id: string;
  kind: Kind;
  registry_id: string;
  registry_index: string;
  version: string;
  distribution: { kind?: string; package?: string; archive?: string; url?: string };
}

const KIND_WORDS: Record<Kind, string> = {
  agent: "agent",
  server: "MCP server",
  skill: "skill",
  tool: "tool",
};

function post<T>(path: string, body: unknown): Promise<T> {
  return fetchJson<T>(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

export function StoreSpace({ hidden }: { hidden: boolean }) {
  const [view, setView] = useState<StoreView | null>(null);
  const [error, setError] = useState("");
  const [search, setSearch] = useState("");
  const [kind, setKind] = useState<Kind | "all">("all");
  const [indexUrl, setIndexUrl] = useState("");
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);

  const load = async () => {
    try {
      setView(await fetchJson<StoreView>("/api/store"));
      setError("");
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
    }
  };
  useEffect(() => {
    if (!hidden) void load();
  }, [hidden]);

  const addIndex = async () => {
    const url = indexUrl.trim();
    if (!url) return;
    setBusy(true);
    setStatus("Reading the catalog…");
    try {
      const added = await post<StoreIndex>("/api/store/indexes", { url });
      setIndexUrl("");
      setStatus(`Added ${added.name}: ${added.entries} entries.`);
      await load();
    } catch (failure) {
      setStatus(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setBusy(false);
    }
  };

  const forgetIndex = async (slug: string) => {
    setBusy(true);
    try {
      await fetchJson(`/api/store/indexes/${encodeURIComponent(slug)}`, { method: "DELETE" });
      setStatus("Forgotten. What was installed from it stays installed.");
      await load();
    } catch (failure) {
      setStatus(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setBusy(false);
    }
  };

  /// Read the exact plan, ask, install against it, and put the result where
  /// the rest of the product reads it from.
  const install = async (entry: StoreEntry) => {
    setBusy(true);
    setStatus(`Reading the plan for ${entry.name}…`);
    try {
      const plan = await post<InstallPlan>("/api/store/plan", { kind: entry.kind, id: entry.id });
      const what = plan.distribution.package ?? plan.distribution.archive ?? plan.distribution.url ?? "its distribution";
      const question =
        `Install ${KIND_WORDS[entry.kind]} ${plan.registry_id} ${plan.version}?\n\n` +
        `It fetches ${what}, described by ${plan.registry_index}, ` +
        "under this product's install root. Nothing else on this machine changes.";
      if (!window.confirm(question)) {
        setStatus("");
        return;
      }
      setStatus(`Installing ${entry.name}…`);
      await post("/api/store/install", { kind: entry.kind, id: entry.id, plan_id: plan.plan_id });
      const after =
        entry.kind === "agent"
          ? "It is offered in the Agent space now."
          : entry.kind === "server"
            ? "It is declared; attach it to an agent in Setup."
            : "A profile can take it in Setup, under Skills.";
      setStatus(`Installed ${entry.name}. ${after}`);
      if (entry.kind === "agent") await sessionStore.loadOnboarding();
      if (entry.kind === "server") await sessionStore.loadMcpServers();
      await load();
    } catch (failure) {
      setStatus(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setBusy(false);
    }
  };

  const needle = search.trim().toLowerCase();
  const entries = (view?.entries ?? []).filter(
    (entry) =>
      (kind === "all" || entry.kind === kind) &&
      (!needle ||
        entry.id.toLowerCase().includes(needle) ||
        entry.name.toLowerCase().includes(needle) ||
        entry.description.toLowerCase().includes(needle)),
  );

  return (
    <div id="store-space" className="space store-space" data-workbench-space="store" hidden={hidden}>
      <aside className="store-sources">
        <div className="k-eyebrow">Where it comes from</div>
        <p className="k-caption k-muted">
          Agents come from the ACP registry. Servers and skills come from catalogs you add by
          address; nothing here is hosted by SWEM.
        </p>
        <div id="registry-status" className="k-caption" data-read={view?.registry.read ?? ""}>
          {view
            ? `ACP registry · ${view.registry.agents} agents · ${view.registry.read}${view.registry.error ? ` (${view.registry.error})` : ""}`
            : "Reading…"}
        </div>
        <ul id="store-indexes" className="server-list">
          {(view?.indexes ?? []).map((index) => (
            <li className="index-row" data-index={index.slug} data-builtin={index.builtin ? "true" : "false"} key={index.slug}>
              <strong>{index.name}</strong>
              <span className="k-caption k-muted">
                {index.entries} entries · {index.builtin ? "came with SWEM" : index.url}
              </span>
              {index.builtin ? null : (
                <button className="index-forget" data-index={index.slug} disabled={busy} onClick={() => void forgetIndex(index.slug)}>
                  Forget
                </button>
              )}
            </li>
          ))}
        </ul>
        <div className="declare-server">
          <input
            id="index-url"
            placeholder="https://…/catalog.json"
            aria-label="Catalog address"
            value={indexUrl}
            onChange={(event) => setIndexUrl(event.target.value)}
          />
          <button id="index-add" disabled={busy || !indexUrl.trim()} onClick={() => void addIndex()}>
            Add catalog
          </button>
        </div>
      </aside>
      <section className="store-list">
        <div className="store-toolbar">
          <input
            id="store-search"
            placeholder="Search agents, servers and skills"
            aria-label="Search the store"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
          <select id="store-kind" aria-label="Kind" value={kind} onChange={(event) => setKind(event.target.value as Kind | "all")}>
            <option value="all">Everything</option>
            <option value="agent">Agents</option>
            <option value="server">MCP servers</option>
            <option value="skill">Skills</option>
          </select>
        </div>
        {error ? <div className="bad k-caption">{error}</div> : null}
        <div id="store-entries" className="rows">
          {entries.map((entry) => (
            <div className="store-entry k-row" data-kind={entry.kind} data-id={entry.id} data-installed={entry.installed ? "true" : "false"} data-bundled={entry.bundled ? "true" : "false"} key={`${entry.kind}:${entry.id}`}>
              <div className="store-entry-words">
                <strong>{entry.name}</strong>
                <small>
                  {KIND_WORDS[entry.kind]} · {entry.id} · {entry.version} · {entry.index}
                  {entry.needs && entry.needs.length > 0 ? ` · needs ${entry.needs.join(", ")}` : ""}
                </small>
                {entry.description ? <span className="k-caption k-muted">{entry.description}</span> : null}
                {!entry.installable && entry.reason ? <span className="k-caption bad">{entry.reason}</span> : null}
              </div>
              <button
                className={entry.installed || entry.bundled ? "store-install" : "store-install primary"}
                data-kind={entry.kind}
                data-id={entry.id}
                disabled={busy || !entry.installable}
                onClick={() => void install(entry)}
              >
                {entry.bundled ? "Came with SWEM" : entry.installed ? `Installed ${entry.installed}` : "Install"}
              </button>
            </div>
          ))}
          {view && entries.length === 0 ? <div className="k-caption k-muted">Nothing matches.</div> : null}
        </div>
        <div id="store-status" className="k-caption">
          {status}
        </div>
      </section>
    </div>
  );
}
