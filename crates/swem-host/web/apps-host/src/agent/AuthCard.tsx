// Getting an agent authorised, as a flow a person can complete on the page.
//
// The card is driven by what the agent's own `initialize` advertised: a
// method that needs values the person types becomes a form under the exact
// variable names the agent asked for; one that needs a login run elsewhere
// becomes an instruction; one that needs nothing is just chosen. The values
// go to the host's typed vault once and never come back to this page.

import { useEffect, useState } from "react";

import type { AuthMethod, AuthVariable, ProfileSecrets, SecretType } from "./session.ts";
import { sessionStore, useSession } from "./store.ts";

const GENERIC = "generic_env_var";

/// The vault kind whose variable the agent named, or the generic kind.
function typeFor(variable: AuthVariable, types: SecretType[]): SecretType {
  return (
    types.find((type) => type.env_var === variable.name) ??
    types.find((type) => type.type_id === GENERIC) ?? { type_id: GENERIC, label: "Environment variable", env_var: null, hint: "" }
  );
}

function VariableRow({ profileId, variable, vault }: { profileId: string; variable: AuthVariable; vault: ProfileSecrets }) {
  const [value, setValue] = useState("");
  const [error, setError] = useState("");
  const present = vault.secrets.some((secret) => secret.name === variable.name);
  const type = typeFor(variable, vault.types);
  const save = async () => {
    setError("");
    try {
      await sessionStore.saveSecret(profileId, {
        type_id: type.type_id,
        label: variable.label ?? type.label,
        name: variable.name,
        value,
      });
      setValue("");
    } catch (failure) {
      setError((failure as Error).message);
    }
  };
  return (
    <div className="auth-var" data-var-name={variable.name} data-present={present ? "true" : "false"}>
      <label htmlFor={`auth-var-${variable.name}`}>
        {variable.label ?? variable.name}
        {variable.optional ? "" : " *"}
        <code>{variable.name}</code>
      </label>
      <div className="auth-var-row">
        <input
          id={`auth-var-${variable.name}`}
          type={variable.secret === false ? "text" : "password"}
          value={value}
          placeholder={present ? "stored — paste again to replace" : type.hint || "paste the value"}
          autoComplete="off"
          onChange={(event) => setValue(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && value) void save();
          }}
        />
        <button className="auth-var-save" disabled={!value} onClick={() => void save()}>
          Save
        </button>
      </div>
      <small className="auth-var-state">{present ? "stored on this computer" : "not set"}</small>
      {error ? <small className="bad">{error}</small> : null}
    </div>
  );
}

function MethodCard({
  method,
  chosen,
  choose,
  profileId,
  vault,
}: {
  method: AuthMethod;
  chosen: boolean;
  choose: () => void;
  profileId: string;
  vault: ProfileSecrets;
}) {
  return (
    <div className="auth-method k-card" data-method-id={method.id} data-chosen={chosen ? "true" : "false"}>
      <label className="auth-method-title">
        <input type="radio" name="auth-method" checked={chosen} onChange={choose} />
        <strong>{method.name}</strong>
      </label>
      {method.description ? <small>{method.description}</small> : null}
      {method.type === "env_var" && chosen
        ? (method.vars ?? []).map((variable) => (
            <VariableRow key={variable.name} profileId={profileId} variable={variable} vault={vault} />
          ))
        : null}
      {method.type === "terminal" && chosen ? (
        <div className="auth-terminal">
          <small>Run this in a terminal on this computer, then start the session:</small>
          <pre>{(method.args ?? []).join(" ") || method.id}</pre>
        </div>
      ) : null}
      {method.link ? (
        <a href={method.link} target="_blank" rel="noopener noreferrer">
          More about this sign-in
        </a>
      ) : null}
    </div>
  );
}

/// Every key the profile holds, and a form for one the agent did not ask for
/// by name (an `OPENAI_API_KEY` for a tool the agent runs, say).
function Vault({ profileId, vault }: { profileId: string; vault: ProfileSecrets }) {
  const [typeId, setTypeId] = useState(vault.types[0]?.type_id ?? GENERIC);
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const [error, setError] = useState("");
  const type = vault.types.find((candidate) => candidate.type_id === typeId);
  const needsName = !type?.env_var;
  const save = async () => {
    setError("");
    try {
      const secret: { type_id: string; label: string; name?: string; value: string } = {
        type_id: typeId,
        label: type?.label ?? typeId,
        value,
      };
      if (needsName) secret.name = name;
      await sessionStore.saveSecret(profileId, secret);
      setValue("");
      setName("");
    } catch (failure) {
      setError((failure as Error).message);
    }
  };
  return (
    <details className="vault">
      <summary>Keys on this computer ({vault.secrets.length})</summary>
      <ul id="secret-list">
        {vault.secrets.map((secret) => (
          <li className="secret-entry" data-name={secret.name} key={secret.name}>
            <span>
              <strong>{secret.label}</strong> <code>{secret.name}</code>
            </span>
            <button
              className="secret-remove"
              onClick={() => void sessionStore.removeSecret(profileId, secret.name).catch((failure: Error) => setError(failure.message))}
            >
              Forget
            </button>
          </li>
        ))}
      </ul>
      <div id="secret-add" className="secret-add">
        <select id="secret-type" aria-label="Kind of key" value={typeId} onChange={(event) => setTypeId(event.target.value)}>
          {vault.types.map((candidate) => (
            <option value={candidate.type_id} key={candidate.type_id}>
              {candidate.label}
              {candidate.env_var ? ` (${candidate.env_var})` : ""}
            </option>
          ))}
        </select>
        {needsName ? (
          <input
            id="secret-name"
            aria-label="Variable name"
            placeholder="VARIABLE_NAME"
            value={name}
            onChange={(event) => setName(event.target.value.toUpperCase())}
          />
        ) : null}
        <input
          id="secret-value"
          type="password"
          aria-label="Value"
          placeholder={type?.hint || "value"}
          autoComplete="off"
          value={value}
          onChange={(event) => setValue(event.target.value)}
        />
        <button id="secret-save" disabled={!value || (needsName && !name)} onClick={() => void save()}>
          Add
        </button>
      </div>
      <div id="auth-error" className="bad">
        {error}
      </div>
    </details>
  );
}

/// The card, and the method the Start button will name.
export function AuthCard({
  methodId,
  onMethod,
  needsAuthentication,
}: {
  methodId: string;
  onMethod: (methodId: string) => void;
  needsAuthentication: boolean;
}) {
  const state = useSession();
  const { profileId } = state;
  const handshake = state.handshakes[profileId];
  const vault = state.secrets[profileId] ?? { types: [], secrets: [] };
  const methods = handshake && !("error" in handshake) ? handshake.auth_methods : [];
  // The first advertised method is the default choice, exactly as the host
  // would pick it; the person can pick another.
  useEffect(() => {
    if (methods.length > 0 && !methods.some((method) => method.id === methodId)) onMethod(methods[0]?.id ?? "");
    if (methods.length === 0 && methodId) onMethod("");
  }, [methods, methodId, onMethod]);
  if (!profileId) return null;
  let status: string;
  if (!handshake) status = "Asking the agent what it needs…";
  else if ("error" in handshake) status = `Could not reach the agent: ${handshake.error}`;
  else {
    const who = [handshake.agent_name, handshake.agent_version].filter(Boolean).join(" ");
    status = methods.length === 0 ? `${who || "The agent"} needs no sign-in.` : `${who || "The agent"} offers ${methods.length === 1 ? "one way" : `${methods.length} ways`} to sign in.`;
  }
  return (
    <section id="auth-card" className="auth-card" data-needs-auth={needsAuthentication ? "true" : "false"}>
      <div className="k-eyebrow">Access</div>
      <div id="auth-status">{status}</div>
      {needsAuthentication ? <div className="auth-hint">The agent asked to be signed in first. Add what it needs below, then start again.</div> : null}
      <div id="auth-methods">
        {methods.map((method) => (
          <MethodCard
            key={method.id}
            method={method}
            chosen={method.id === methodId}
            choose={() => onMethod(method.id)}
            profileId={profileId}
            vault={vault}
          />
        ))}
      </div>
      <Vault profileId={profileId} vault={vault} />
    </section>
  );
}
