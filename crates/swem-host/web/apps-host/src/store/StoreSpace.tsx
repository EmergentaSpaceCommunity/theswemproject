// The Store: one list of what a person can install, from the indexes this
// product reads - the ACP registry's agents, and the catalogs a person adds
// by URL - for every host that takes something from it: the Workbench takes
// agents, MCP servers and skills; an installed server or a product the
// Workbench is built into takes kinds of its own, and says what to call
// them. A kind nobody here takes is listed as such and cannot be installed.
//
// Installing here is the same act as everywhere in the product: the exact
// plan is read, with what it requires, the person consents to it by name in
// a dialog, and the install is applied against that plan's id. What happens
// afterwards is the taker's, and the taker says it.
import { useEffect, useState } from "react";
import { useStore } from "zustand";

import { sessionStore } from "../agent/store.ts";
import { fetchJson } from "../http.ts";
import { Consent } from "../page/Consent.tsx";
import { named } from "../page/door.ts";

type Kind = string;

interface KindShown {
  kind: Kind;
  one: string;
  many: string;
  after_install: string;
}

interface Requirement {
  kind: Kind;
  id: string;
  version?: string;
}

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
  taken?: boolean;
  requires?: Requirement[];
  newer?: boolean;
  removable?: boolean;
  required_by?: string[];
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
  kinds?: KindShown[];
}

interface InstallPlan {
  plan_id: string;
  kind: Kind;
  registry_id: string;
  registry_index: string;
  version: string;
  distribution: { kind?: string; package?: string; archive?: string; url?: string };
  also?: InstallPlan[];
}

/// What a kind is called: the taker's words, or the kind's own name for one
/// nobody here takes.
const NOBODY: KindShown = { kind: "", one: "something this Workbench does not have", many: "For something this Workbench does not have", after_install: "" };
const wordsFor = (kinds: KindShown[], kind: Kind): KindShown => kinds.find((one) => one.kind === kind) ?? { ...NOBODY, kind };

const fetched = (plan: InstallPlan): string => plan.distribution.package ?? plan.distribution.archive ?? plan.distribution.url ?? "its distribution";

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
  const called = useStore(named, (state) => state.called);
  const [busy, setBusy] = useState(false);
  const [consent, setConsent] = useState<{ asked: string; yes: string; answer: (yes: boolean) => void } | null>(null);

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
      const words = wordsFor(view?.kinds ?? [], entry.kind);
      const also = (plan.also ?? []).map((one) => `${wordsFor(view?.kinds ?? [], one.kind).one} ${one.registry_id} ${one.version} (${fetched(one)})`);
      const question =
        `${entry.installed ? "Update" : "Install"} ${words.one} ${plan.registry_id} ${plan.version}?\n\n` +
        `It fetches ${fetched(plan)}, described by ${plan.registry_index || entry.index}, ` +
        "under this product's install root." +
        (also.length > 0 ? ` It also installs what this requires: ${also.join("; ")}.` : "") +
        " Nothing else on this machine changes.";
      if (!(await new Promise<boolean>((answer) => setConsent({ asked: question, yes: entry.installed ? "Update" : "Install", answer })))) {
        setStatus("");
        return;
      }
      setStatus(`Installing ${entry.name}…`);
      await post("/api/store/install", { kind: entry.kind, id: entry.id, plan_id: plan.plan_id });
      setStatus(`Installed ${entry.name}${also.length > 0 ? ` and what it requires` : ""}. ${words.after_install}`);
      if (entry.kind === "agent") await sessionStore.loadOnboarding();
      if (entry.kind === "server") await sessionStore.loadMcpServers();
      await load();
    } catch (failure) {
      setStatus(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setBusy(false);
    }
  };

  /// Remove what was installed, after the person said so.
  const remove = async (entry: StoreEntry) => {
    const words = wordsFor(view?.kinds ?? [], entry.kind);
    const question = `Remove ${words.one} ${entry.id} ${entry.installed ?? ""}?\n\nWhat was installed of it goes; what was made with it stays.`;
    if (!(await new Promise<boolean>((answer) => setConsent({ asked: question, yes: "Remove", answer })))) return;
    setBusy(true);
    setStatus(`Removing ${entry.name}…`);
    try {
      await post("/api/store/remove", { kind: entry.kind, id: entry.id });
      setStatus(`Removed ${entry.name}.`);
      if (entry.kind === "server") await sessionStore.loadMcpServers();
      await load();
    } catch (failure) {
      setStatus(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setBusy(false);
    }
  };

  const kinds = view?.kinds ?? [];
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
      <Consent
        asked={consent?.asked ?? null}
        yes={consent?.yes}
        onAnswer={(yes) => {
          consent?.answer(yes);
          setConsent(null);
        }}
      />
      <aside className="store-sources">
        <div className="k-eyebrow">Where it comes from</div>
        <p className="k-caption k-muted">
          Agents come from the ACP registry. The rest comes from catalogs you add by address;
          nothing here is hosted by {called}. {kinds.length > 0 ? `Here: ${kinds.map((one) => one.many.toLowerCase()).join(", ")}.` : ""}
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
                {index.entries} entries · {index.builtin ? `came with ${called}` : index.url}
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
            placeholder="Search"
            aria-label="Search the store"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
          <select id="store-kind" aria-label="Kind" value={kind} onChange={(event) => setKind(event.target.value as Kind | "all")}>
            <option value="all">Everything</option>
            {kinds.map((one) => (
              <option value={one.kind} key={one.kind}>
                {one.many}
              </option>
            ))}
          </select>
        </div>
        {error ? <div className="bad k-caption">{error}</div> : null}
        <div id="store-entries" className="rows">
          {entries.map((entry) => (
            <div className="store-entry k-row" data-kind={entry.kind} data-id={entry.id} data-installed={entry.installed ? "true" : "false"} data-bundled={entry.bundled ? "true" : "false"} key={`${entry.kind}:${entry.id}`}>
              <div className="store-entry-words">
                <strong>{entry.name}</strong>
                <small>
                  {wordsFor(kinds, entry.kind).one} · {entry.id} · {entry.version} · {entry.index}
                  {entry.needs && entry.needs.length > 0 ? ` · needs ${entry.needs.join(", ")}` : ""}
                  {entry.requires && entry.requires.length > 0 ? ` · requires ${entry.requires.map((one) => one.id).join(", ")}` : ""}
                  {entry.required_by && entry.required_by.length > 0 ? ` · required by ${entry.required_by.map((one) => one.split(" ").pop()).join(", ")}` : ""}
                </small>
                {entry.description ? <span className="k-caption k-muted">{entry.description}</span> : null}
                {!entry.installable && entry.reason ? <span className="k-caption bad">{entry.reason}</span> : null}
              </div>
              <span className="k-inline w-tight w-nowrap">
                {entry.installed && entry.removable ? (
                  <button className="store-remove k-quiet" data-kind={entry.kind} data-id={entry.id} disabled={busy} onClick={() => void remove(entry)}>
                    Remove
                  </button>
                ) : null}
                <button
                  className={entry.installed || entry.bundled ? "store-install" : "store-install primary"}
                  data-kind={entry.kind}
                  data-id={entry.id}
                  disabled={busy || !entry.installable || (Boolean(entry.installed) && !entry.newer)}
                  onClick={() => void install(entry)}
                >
                  {entry.bundled ? `Came with ${called}` : entry.installed ? (entry.newer ? `Update to ${entry.version}` : `Installed ${entry.installed}`) : "Install"}
                </button>
              </span>
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
