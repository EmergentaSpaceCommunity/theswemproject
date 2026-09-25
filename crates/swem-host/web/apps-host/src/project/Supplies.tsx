// What a realization needs besides code, from the product: the tools
// packages declare (installed here from the exact plan shown, into the
// product's own directory - never by hand on the machine), and the secrets a
// project's nodes may require (kept in the project's vault by type; the
// entry is what a binding names, the value never leaves the host).
import { useEffect, useState } from "react";

import { fetchJson } from "./api";

interface ToolView {
  name: string;
  module: string;
  version: string;
  platform: string;
  plan_id: string;
  url: string;
  sha256: string;
  entry: string;
  executable?: string;
}

interface SecretTypeEntry {
  type_id: string;
  module: string;
  label: string;
  env_var?: string;
  hint?: string;
}

interface ProjectSecrets {
  types: SecretTypeEntry[];
  entries: { vault_ref: string; type_id: string }[];
}

export function Tools() {
  const [tools, setTools] = useState<ToolView[]>([]);
  const [status, setStatus] = useState("");
  const load = () =>
    fetchJson<ToolView[]>("/api/tools")
      .then((list) => {
        setTools(list);
        setStatus("");
      })
      .catch((error: unknown) => setStatus(String(error)));
  useEffect(() => {
    void load();
  }, []);
  const install = async (tool: ToolView) => {
    // The plan is what the person consents to: the exact archive, digest
    // and version, shown before the click.
    const question = `Install ${tool.name} ${tool.version} for ${tool.module}?\n\n${tool.url}\nsha256 ${tool.sha256}`;
    if (!window.confirm(question)) return;
    setStatus(`Installing ${tool.name}…`);
    try {
      await fetchJson<ToolView>(`/api/tools/${encodeURIComponent(tool.name)}/install`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ plan_id: tool.plan_id }),
      });
      await load();
    } catch (error) {
      setStatus(String(error));
    }
  };
  if (tools.length === 0 && !status) return null;
  return (
    <section id="project-tools">
      <h3>Tools packages bring</h3>
      <div className="rows">
        {tools.map((tool) => (
          <div key={tool.name} className="tool-row k-row" data-tool={tool.name} data-installed={tool.executable ? "true" : "false"}>
            <strong>{tool.name}</strong>
            <small>
              {tool.version} · {tool.module}
            </small>
            {tool.executable ? (
              <code className="tool-path">{tool.executable}</code>
            ) : (
              <button className="tool-install" data-tool={tool.name} onClick={() => void install(tool)}>
                Install
              </button>
            )}
          </div>
        ))}
      </div>
      <div id="tools-status" className="k-caption">
        {status}
      </div>
    </section>
  );
}

export function Secrets({ serverName }: { serverName: string }) {
  const [secrets, setSecrets] = useState<ProjectSecrets | null>(null);
  const [status, setStatus] = useState("");
  const [typeId, setTypeId] = useState("");
  const [value, setValue] = useState("");
  // `quiet` is for the re-read that follows a keeping. Without it the list
  // coming back erased the very line that said what had just been kept, so
  // pressing Keep looked like pressing nothing.
  const load = (quiet = false) =>
    fetchJson<ProjectSecrets>(`/api/projects/${encodeURIComponent(serverName)}/secrets`)
      .then((loaded) => {
        setSecrets(loaded);
        if (!quiet) setStatus("");
        if (!typeId && loaded.types.length > 0) setTypeId(loaded.types[0].type_id);
      })
      .catch(() => setSecrets(null));
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serverName]);
  const store = async () => {
    if (!typeId || !value) return;
    setStatus("Keeping it…");
    try {
      const entry = await fetchJson<{ vault_ref: string }>(
        `/api/projects/${encodeURIComponent(serverName)}/secrets/${encodeURIComponent(typeId)}`,
        { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify({ value }) },
      );
      setValue("");
      setStatus(`Kept as ${entry.vault_ref}; bind it to the node that requires it.`);
      await load(true);
    } catch (error) {
      setStatus(error instanceof Error ? error.message : String(error));
    }
  };
  if (secrets === null || secrets.types.length === 0) return null;
  return (
    <section id="project-secrets">
      <h3>Secrets</h3>
      <p className="k-caption k-muted">
        A node that requires a secret names its type; the value is kept here, in this project's
        vault, and reaches the release when the delivery closes. It is never written to a record.
      </p>
      <div className="rows">
        {secrets.entries.map((entry) => (
          <div key={entry.vault_ref} className="secret-row k-row" data-vault-ref={entry.vault_ref} data-type-id={entry.type_id}>
            <code>{entry.vault_ref}</code>
            <small>{entry.type_id}</small>
          </div>
        ))}
      </div>
      <div className="pair">
        <select id="secret-type" aria-label="Secret type" value={typeId} onChange={(event) => setTypeId(event.target.value)}>
          {secrets.types.map((kind) => (
            <option key={kind.type_id} value={kind.type_id}>
              {kind.label} ({kind.type_id})
            </option>
          ))}
        </select>
        <input id="secret-value" type="password" placeholder="its value" value={value} onChange={(event) => setValue(event.target.value)} />
        <button id="secret-keep" disabled={!typeId || !value} onClick={() => void store()}>
          Keep
        </button>
      </div>
      <div id="secrets-status" className="k-caption">
        {status}
      </div>
    </section>
  );
}
