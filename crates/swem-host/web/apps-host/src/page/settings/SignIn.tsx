// How an agent signs in, and what it holds of its own.
//
// What is offered is what the agent's engine said it takes: a way that
// needs values becomes fields under the names the engine asked for, a way
// that needs a login run elsewhere becomes the command to run, a way that
// needs nothing is only chosen. A value goes to the host once and never
// comes back to the page.

import { useEffect, useState } from "react";

import type { AuthMethod, AuthVariable, ProfileSecrets, SecretType } from "../../agent/session.ts";
import { sessionStore, useSession } from "../../agent/store.ts";

const GENERIC = "generic_env_var";

/// The kind of key whose variable the engine named, or the generic kind.
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
      await sessionStore.saveSecret(profileId, { type_id: type.type_id, label: variable.label ?? type.label, name: variable.name, value });
      setValue("");
    } catch (failure) {
      setError((failure as Error).message);
    }
  };
  return (
    <form
      className="auth-var k-stack w-close"
      data-var-name={variable.name}
      data-present={present ? "true" : "false"}
      onSubmit={(event) => {
        event.preventDefault();
        if (value) void save();
      }}
    >
      <label className="k-caption" htmlFor={`auth-var-${variable.name}`}>
        {variable.label ?? variable.name}
        {variable.optional ? " (if you like)" : ""} <code className="k-mono">{variable.name}</code>
      </label>
      <div className="k-inline w-tight w-nowrap">
        <input
          className="k-field k-grow"
          id={`auth-var-${variable.name}`}
          type={variable.secret === false ? "text" : "password"}
          value={value}
          placeholder={present ? "Kept. Paste again to replace it" : type.hint || "Paste the value"}
          autoComplete="off"
          onChange={(event) => setValue(event.target.value)}
        />
        <button type="submit" className="k-btn auth-var-save" disabled={!value}>
          Keep
        </button>
      </div>
      <span className="k-caption">{present ? "Kept on this computer, for this agent alone" : "Not given"}</span>
      {error ? <span className="k-caption k-is-danger">{error}</span> : null}
    </form>
  );
}

function Way({
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
    <div className={`auth-method k-card w-inset${chosen ? " k-active" : ""}`} data-method-id={method.id} data-chosen={chosen ? "true" : "false"}>
      <label className="k-inline w-tight">
        <input type="radio" name="auth-method" checked={chosen} onChange={choose} />
        <span className="k-name">{method.name}</span>
      </label>
      {method.description ? <span className="k-caption">{method.description}</span> : null}
      {method.type === "env_var" && chosen
        ? (method.vars ?? []).map((variable) => <VariableRow key={variable.name} profileId={profileId} variable={variable} vault={vault} />)
        : null}
      {method.type === "terminal" && chosen ? (
        <div className="k-stack w-close">
          <span className="k-caption">Run this in the agent's Terminal, then write to it:</span>
          <pre className="k-code">{(method.args ?? []).join(" ") || method.id}</pre>
        </div>
      ) : null}
      {method.link ? (
        <a href={method.link} target="_blank" rel="noopener noreferrer">
          More about this way of signing in
        </a>
      ) : null}
    </div>
  );
}

/// What this agent holds that no other agent is handed: a key of its own in
/// place of its provider's, a token one of its servers reads.
function ItsOwn({ profileId, vault }: { profileId: string; vault: ProfileSecrets }) {
  const [typeId, setTypeId] = useState(vault.types[0]?.type_id ?? GENERIC);
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const [error, setError] = useState("");
  const type = vault.types.find((candidate) => candidate.type_id === typeId);
  const needsName = !type?.env_var;
  const save = async () => {
    setError("");
    try {
      const secret: { type_id: string; label: string; name?: string; value: string } = { type_id: typeId, label: type?.label ?? typeId, value };
      if (needsName) secret.name = name;
      await sessionStore.saveSecret(profileId, secret);
      setValue("");
      setName("");
    } catch (failure) {
      setError((failure as Error).message);
    }
  };
  return (
    <div className="k-stack">
      <span className="k-eyebrow">What this agent holds of its own</span>
      <span className="k-caption">
        Handed to this agent alone, over what its provider was given. A key for every agent is given once, in Providers.
      </span>
      <div id="secret-list">
        {vault.secrets.map((secret) => (
          <div className="secret-entry k-row" data-name={secret.name} key={secret.name}>
            <span className="k-two k-grow">
              <span className="k-name">{secret.label}</span>
              <span className="k-caption k-mono">{secret.name}</span>
            </span>
            <button
              type="button"
              className="k-btn k-quiet secret-remove"
              onClick={() => void sessionStore.removeSecret(profileId, secret.name).catch((failure: Error) => setError(failure.message))}
            >
              Take away
            </button>
          </div>
        ))}
        {vault.secrets.length === 0 ? <span className="k-caption">Nothing.</span> : null}
      </div>
      <form
        id="secret-add"
        className="k-inline w-tight w-top"
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <select className="k-field" id="secret-type" aria-label="What kind it is" value={typeId} onChange={(event) => setTypeId(event.target.value)}>
          {vault.types.map((candidate) => (
            <option value={candidate.type_id} key={candidate.type_id}>
              {candidate.label}
            </option>
          ))}
        </select>
        {needsName ? (
          <input
            className="k-field k-mono"
            id="secret-name"
            aria-label="The variable it is read from"
            placeholder="VARIABLE_NAME"
            value={name}
            onChange={(event) => setName(event.target.value.toUpperCase())}
          />
        ) : null}
        <input
          className="k-field k-grow"
          id="secret-value"
          type="password"
          aria-label="Its value"
          placeholder={type?.hint || "Its value"}
          autoComplete="off"
          value={value}
          onChange={(event) => setValue(event.target.value)}
        />
        <button type="submit" id="secret-save" className="k-btn" disabled={!value || (needsName && !name)}>
          Keep it
        </button>
      </form>
      {error ? (
        <span id="auth-error" className="k-caption k-is-danger">
          {error}
        </span>
      ) : null}
    </div>
  );
}

export function SignIn({ needsSignIn }: { needsSignIn: boolean }) {
  const state = useSession();
  const profileId = state.profileId;
  const handshake = state.handshakes[profileId];
  const vault = state.secrets[profileId] ?? { types: [], secrets: [] };
  const methods = handshake && !("error" in handshake) ? handshake.auth_methods : [];
  const chosen = state.authMethodId;
  // The first way the engine names is the one chosen, as the host would
  // choose it; the person can choose another.
  useEffect(() => {
    if (methods.length > 0 && !methods.some((method) => method.id === chosen)) sessionStore.setAuthMethod(methods[0]?.id ?? "");
    if (methods.length === 0 && chosen) sessionStore.setAuthMethod("");
  }, [methods, chosen]);
  return (
    <div id="auth-card" className="k-stack" data-needs-auth={needsSignIn ? "true" : "false"}>
      {needsSignIn ? <div className="k-notice k-warning">This agent could not answer: its engine asked to be signed in.</div> : null}
      <span id="auth-status" className="k-caption">
        {handshake === undefined
          ? "Asking the engine how it signs in…"
          : "error" in handshake
            ? `The engine could not be asked how it signs in: ${handshake.error}`
            : methods.length === 0
              ? "Its engine asks for no sign-in."
              : "The ways its engine signs in. The one chosen is used when it next starts."}
      </span>
      <div id="auth-methods" className="k-stack">
        {methods.map((method) => (
          <Way
            key={method.id}
            method={method}
            chosen={chosen === method.id}
            choose={() => sessionStore.setAuthMethod(method.id)}
            profileId={profileId}
            vault={vault}
          />
        ))}
      </div>
      <ItsOwn profileId={profileId} vault={vault} />
    </div>
  );
}
