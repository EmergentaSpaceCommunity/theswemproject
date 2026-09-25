// The packages this product is running. A domain, a language profile or a
// catalogue enters the Cycle as a package, and until now one
// arrived only by being put in the data root by hand. Here a person names a
// source, reads what it would install and what that package declares, and
// consents to that exact plan. What is installed reaches every project
// opened afterwards; the plan is read again if the source moves under it.
//
// A product reads packages from more than one place - the ones its
// distribution ships beside the binary, and the ones this person installed -
// and this panel used to list only the second. On the distribution everyone
// actually runs, that meant a person looking at a project made from a
// package's own seed read "Packages (0)".
import { useEffect, useState } from "react";

import { fetchJson } from "./api";

interface PackageDeclares {
  behaviour: string;
  node_types: string[];
  adapters: string[];
  tools: string[];
  secret_types: string[];
  language_profiles: string[];
  predicates: string[];
  resources: number;
  prompts: number;
  realizations: number;
  recipes: number;
  seeds: number;
  door?: string;
}

interface PackageSource {
  kind: "directory" | "archive";
  path?: string;
  url?: string;
  sha256?: string;
}

interface PackageView {
  id: string;
  /// "product" - it came with this product; "yours" - this person installed
  /// it, and it is theirs to replace.
  home?: "product" | "yours";
  version: string;
  title: string;
  summary: string;
  directory: string;
  declares: PackageDeclares;
  source?: PackageSource;
  installed_at?: number;
  refused?: string;
}

interface PackagePlan {
  plan_id: string;
  id: string;
  version: string;
  title: string;
  summary: string;
  source: PackageSource;
  sha256: string;
  declares: PackageDeclares;
  replaces?: string;
}

/// What a package brings, as a line a person reads rather than a manifest.
/// Counts where the count is the meaning, names where the name is.
function brings(declares: PackageDeclares): string {
  const parts: string[] = [];
  if (declares.node_types.length > 0) parts.push(`${declares.node_types.length} node types`);
  if (declares.door) parts.push("a door of its own");
  if (declares.adapters.length > 0) parts.push(`${declares.adapters.length} adapters`);
  if (declares.tools.length > 0) parts.push(`tools: ${declares.tools.join(", ")}`);
  if (declares.language_profiles.length > 0) parts.push(declares.language_profiles.join(", "));
  if (declares.realizations > 0) parts.push(`${declares.realizations} realizations`);
  if (declares.resources > 0) parts.push(`${declares.resources} resources`);
  if (declares.recipes > 0) {
    const seeds = declares.seeds > 0 ? `, ${declares.seeds} to start a project from` : "";
    parts.push(`${declares.recipes} recipes${seeds}`);
  }
  if (parts.length === 0) parts.push("data only");
  return parts.join(" · ");
}

interface Props {
  /// A package is how a recipe arrives, so whoever offers those reads them
  /// again once one is installed.
  onInstalled: () => void;
}

export function Packages({ onInstalled }: Props) {
  const [packages, setPackages] = useState<PackageView[]>([]);
  const [kind, setKind] = useState<"directory" | "archive">("directory");
  const [path, setPath] = useState("");
  const [url, setUrl] = useState("");
  const [sha256, setSha256] = useState("");
  const [plan, setPlan] = useState<PackagePlan | null>(null);
  const [status, setStatus] = useState("");
  const load = () =>
    fetchJson<PackageView[]>("/api/packages")
      .then((list) => setPackages(list))
      .catch(() => setPackages([]));
  useEffect(() => {
    void load();
  }, []);

  const readPlan = async () => {
    const source: PackageSource =
      kind === "directory" ? { kind, path: path.trim() } : { kind, url: url.trim(), sha256: sha256.trim() };
    setStatus("Reading the source…");
    setPlan(null);
    try {
      const read = await fetchJson<PackagePlan>("/api/packages/plan", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(source),
      });
      setPlan(read);
      setStatus("");
    } catch (error) {
      setStatus(error instanceof Error ? error.message : String(error));
    }
  };

  const install = async () => {
    if (!plan) return;
    setStatus(`Installing ${plan.id}…`);
    try {
      await fetchJson<PackageView>("/api/packages/install", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ plan_id: plan.plan_id }),
      });
      setPlan(null);
      setPath("");
      setUrl("");
      setSha256("");
      setStatus("Installed. A project opened from now on has it.");
      await load();
      onInstalled();
    } catch (error) {
      setStatus(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <details id="packages">
      <summary>Packages ({packages.length})</summary>
      <div className="packages-body">
        <p className="k-caption k-muted">
          A domain, a language profile or a catalogue is a package. The ones that came with SWEM
          are always here; what you install is loaded by every project opened afterwards.
        </p>
        <div className="rows">
          {packages.map((installed) => (
            <div
              key={`${installed.home ?? "product"}:${installed.id}`}
              className="package-row k-row"
              data-package={installed.id}
              data-package-home={installed.home ?? "product"}
              data-refused={installed.refused ? "true" : "false"}
            >
              <strong>{installed.title || installed.id}</strong>
              <small>
                {installed.version} ·{" "}
                {installed.home === "yours" ? "installed here" : "came with SWEM"} ·{" "}
                {brings(installed.declares)}
              </small>
              {installed.refused ? <small className="bad">{installed.refused}</small> : null}
            </div>
          ))}
        </div>
        <div className="package-source">
          <select
            id="package-source-kind"
            aria-label="Where the package comes from"
            value={kind}
            onChange={(event) => {
              setKind(event.target.value === "archive" ? "archive" : "directory");
              setPlan(null);
            }}
          >
            <option value="directory">A directory on this machine</option>
            <option value="archive">An archive, by url and digest</option>
          </select>
          {kind === "directory" ? (
            <input
              id="package-path"
              placeholder="path to the package directory"
              aria-label="Package directory"
              value={path}
              onChange={(event) => setPath(event.target.value)}
            />
          ) : (
            <>
              <input
                id="package-url"
                placeholder="https://…/package.tar.gz"
                aria-label="Package archive url"
                value={url}
                onChange={(event) => setUrl(event.target.value)}
              />
              <input
                id="package-sha256"
                placeholder="its sha256"
                aria-label="Package archive digest"
                value={sha256}
                onChange={(event) => setSha256(event.target.value)}
              />
            </>
          )}
          <button
            id="package-plan"
            disabled={kind === "directory" ? !path.trim() : !url.trim() || !sha256.trim()}
            onClick={() => void readPlan()}
          >
            Read it
          </button>
        </div>
        {plan ? (
          <div id="package-plan-card" className="package-plan" data-package={plan.id}>
            <strong>
              {plan.title || plan.id} {plan.version}
            </strong>
            <p className="k-caption">{plan.summary}</p>
            <p className="k-caption">It brings {brings(plan.declares)}.</p>
            {plan.replaces ? (
              <p className="k-caption bad" id="package-replaces">
                This replaces {plan.id} {plan.replaces}, which is installed now.
              </p>
            ) : null}
            <code className="k-caption">{plan.sha256}</code>
            <button id="package-install" onClick={() => void install()}>
              Install {plan.id}
            </button>
          </div>
        ) : null}
        <div id="packages-status" className="k-caption">
          {status}
        </div>
      </div>
    </details>
  );
}
