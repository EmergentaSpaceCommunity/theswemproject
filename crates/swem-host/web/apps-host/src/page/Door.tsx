// The door: what whoever has not come in yet is shown. A Workbench on the
// computer a person sits at has none and draws itself at once; one served
// at an address draws the first start, sign-in, or the way back with a code.

import { useCallback, useEffect, useState, type FormEvent, type ReactNode } from "react";

import { SIGN_IN_ASKED } from "../http.ts";
import { useStore } from "zustand";

import { comeBack, doorStanding, likelyName, named, register, signIn, wordForADevice, type DoorStanding } from "./door.ts";
import { Check, Key, Lock } from "./icons.tsx";
import { ThemeSwitch } from "./Rail.tsx";

const STEPS = ["The word", "This device", "Codes to come back with"];

function Steps({ at }: { at: number }) {
  return (
    <ol className="w-steps-down" aria-label="Steps">
      {STEPS.map((step, index) => (
        <li className={`w-step-across${index === at ? " k-active" : ""}${index < at ? " w-done" : ""}`} aria-current={index === at ? "step" : undefined} key={step}>
          <span className="w-step-number">{index < at ? <Check size={13} /> : index + 1}</span>
          <span>{step}</span>
        </li>
      ))}
    </ol>
  );
}

function Alone({ children }: { children: ReactNode }) {
  const called = useStore(named, (state) => state.called);
  return (
    <div className="w-shell w-alone">
      <div className="w-alone-theme">
        <ThemeSwitch />
      </div>
      <div className="w-brand">
        <span className="w-brand-mark" />
        <span>{called}</span>
      </div>
      {children}
    </div>
  );
}

const told = (error: unknown): string => (error instanceof Error ? error.message : String(error));

/// The codes, said once.
export function Codes({ codes }: { codes: string[] }) {
  const [copied, setCopied] = useState(false);
  const text = codes.join("\n");
  const copy = () => {
    void navigator.clipboard?.writeText(text).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    });
  };
  const file = `data:text/plain;charset=utf-8,${encodeURIComponent(`Codes to come back to the SWEM Workbench at ${window.location.host}\nEach is used once.\n\n${text}\n`)}`;
  return (
    <>
      <ul className="w-codes" aria-label="Codes to come back with">
        {codes.map((code) => (
          <li className="w-code" key={code}>
            {code}
          </li>
        ))}
      </ul>
      <div className="k-inline w-tight">
        <button type="button" className="k-btn w-tall" onClick={copy}>
          {copied ? "Copied" : "Copy"}
        </button>
        <a className="k-btn w-tall" href={file} download="swem-codes-to-come-back-with.txt">
          Save as a file
        </a>
      </div>
      <div className="k-notice k-warning">The Workbench keeps these only in a form it cannot read back. It cannot show them again.</div>
    </>
  );
}

/// Registering a device: with the word of a first start, with a word made
/// for one more device, or by whoever came back with a code.
function Register({ standing, first, withACode, onIn, onBack }: { standing: DoorStanding; first: boolean; withACode: boolean; onIn: (codes: string[] | null) => void; onBack?: () => void }) {
  const [word, setWord] = useState("");
  const [name, setName] = useState(likelyName);
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setProblem("");
    try {
      const given = withACode ? (await wordForADevice()).word : word;
      const came = await register(given, name);
      onIn(came.codes);
    } catch (error) {
      setProblem(told(error));
    } finally {
      setBusy(false);
    }
  };
  const title = first ? "Make this Workbench yours" : "Register this device";
  const about = first
    ? "It was started for the first time and belongs to nobody yet. The word it printed where you started it is used once: it shows that it is you who started it."
    : withACode
      ? "You came back with a code. Register this device, so that you come in with it from now on."
      : "A word is made for one more device under Settings, Access, on a device that is signed in. It is used once and is good for ten minutes.";
  return (
    <div className="w-door-row">
      {first ? <Steps at={0} /> : null}
      <form className="k-dialog w-door" onSubmit={(event) => void submit(event)} aria-label={title}>
        <div className="w-col w-close">
          <h1 className="w-h1">{title}</h1>
          <span className="k-caption">{about}</span>
        </div>
        {withACode ? null : (
          <label className="w-col w-close">
            <span className="k-caption">{first ? "The word it printed" : "The word"}</span>
            <input className="k-field k-mono" value={word} onChange={(event) => setWord(event.target.value)} autoComplete="off" autoCapitalize="none" spellCheck={false} placeholder="xxxx-xxxx-xxxx-xxxx" required autoFocus />
          </label>
        )}
        <label className="w-col w-close">
          <span className="k-caption">A name for this device</span>
          <input className="k-field" value={name} onChange={(event) => setName(event.target.value)} maxLength={60} required />
          <span className="k-caption">So that you can tell your devices apart later.</span>
        </label>
        <div className="k-notice">Your browser will ask for your fingerprint, your face or the lock of your screen. The key stays in this device; the Workbench keeps only the half that checks it.</div>
        {problem ? (
          <div className="k-notice k-danger" role="alert">
            {problem}
          </div>
        ) : null}
        <div className="k-spread">
          <span className="k-caption k-mono">{standing.name}</span>
          <span className="k-inline w-tight">
            {onBack ? (
              <button type="button" className="k-btn k-quiet w-tall" onClick={onBack}>
                Back
              </button>
            ) : null}
            <button type="submit" className="k-btn k-primary w-tall" disabled={busy || !name.trim() || (!withACode && !word.trim())}>
              <Key size={16} />
              <span>{busy ? "Registering" : "Register this device"}</span>
            </button>
          </span>
        </div>
      </form>
    </div>
  );
}

function CodesOnce({ codes, onDone }: { codes: string[]; onDone: () => void }) {
  const [kept, setKept] = useState(false);
  return (
    <div className="w-door-row">
      <Steps at={2} />
      <section className="k-dialog w-door" aria-label="Codes to come back with">
        <div className="w-col w-close">
          <h1 className="w-h1">Codes to come back with</h1>
          <span className="k-caption">Shown once. If every device of yours is lost, one of these lets you in to register a new one. Each is used once.</span>
        </div>
        <Codes codes={codes} />
        <div className="k-spread">
          <label className="k-inline w-tight">
            <input type="checkbox" checked={kept} onChange={(event) => setKept(event.target.checked)} />
            <span>I have kept them somewhere safe</span>
          </label>
          <button type="button" className="k-btn k-primary w-tall" disabled={!kept} onClick={onDone}>
            Open the Workbench
          </button>
        </div>
      </section>
    </div>
  );
}

function SignIn({ standing, onIn, onWord }: { standing: DoorStanding; onIn: () => void; onWord: () => void }) {
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const [withCode, setWithCode] = useState(false);
  const [code, setCode] = useState("");
  const come = async (how: () => Promise<unknown>) => {
    setBusy(true);
    setProblem("");
    try {
      await how();
      onIn();
    } catch (error) {
      setProblem(told(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <section className="k-dialog w-door w-narrow" aria-label="Sign in">
        <div className="w-col w-close">
          <h1 className="w-h1">Sign in</h1>
          <span className="k-caption k-mono">{standing.name}</span>
        </div>
        {withCode ? (
          <form
            className="w-col"
            onSubmit={(event) => {
              event.preventDefault();
              void come(() => comeBack(code));
            }}
          >
            <label className="w-col w-close">
              <span className="k-caption">One of the codes you were shown when you made this Workbench yours</span>
              <input className="k-field k-mono" value={code} onChange={(event) => setCode(event.target.value)} autoComplete="off" autoCapitalize="none" spellCheck={false} placeholder="xxxx-xxxx-xxxx" required autoFocus />
            </label>
            <div className="k-inline w-end w-tight">
              <button type="button" className="k-btn k-quiet w-tall" onClick={() => setWithCode(false)}>
                Back
              </button>
              <button type="submit" className="k-btn k-primary w-tall" disabled={busy || !code.trim()}>
                Come back
              </button>
            </div>
          </form>
        ) : (
          <>
            <button type="button" className="k-btn k-primary w-tall" disabled={busy} onClick={() => void come(signIn)}>
              <Key size={16} />
              <span>{busy ? "Signing in" : "Sign in with a passkey"}</span>
            </button>
            <div className="k-notice k-inline w-nowrap w-tight">
              <Lock size={15} />
              <span>Only a device that was registered here comes in. There is no password to guess.</span>
            </div>
          </>
        )}
        {problem ? (
          <div className="k-notice k-danger" role="alert">
            {problem}
          </div>
        ) : null}
        {withCode ? null : (
          <div className="w-col w-close">
            <button type="button" className="k-btn k-quiet w-tall" onClick={onWord}>
              Register this device with a word
            </button>
            <button type="button" className="k-btn k-quiet w-tall" onClick={() => setWithCode(true)}>
              Every device is lost: come back with a code
            </button>
          </div>
        )}
      </section>
      <span className="k-caption">Behind this door are your agents, their files and their terminals.</span>
    </>
  );
}

export function Door({ children }: { children: ReactNode }) {
  const [standing, setStanding] = useState<DoorStanding | null>(null);
  const [problem, setProblem] = useState("");
  const [codes, setCodes] = useState<string[] | null>(null);
  const [withWord, setWithWord] = useState(false);
  const ask = useCallback(() => {
    doorStanding()
      .then((now) => {
        setStanding(now);
        setProblem("");
      })
      .catch((error: unknown) => setProblem(told(error)));
  }, []);
  useEffect(() => {
    ask();
    window.addEventListener(SIGN_IN_ASKED, ask);
    return () => window.removeEventListener(SIGN_IN_ASKED, ask);
  }, [ask]);
  if (!standing) {
    return problem ? (
      <Alone>
        <div className="k-notice k-danger w-door w-narrow" role="alert">
          {problem}
        </div>
      </Alone>
    ) : null;
  }
  const who = standing.who;
  if (codes) {
    return (
      <Alone>
        <CodesOnce
          codes={codes}
          onDone={() => {
            setCodes(null);
            ask();
          }}
        />
      </Alone>
    );
  }
  if (standing.at === null || (who !== null && who.by !== "code")) return <>{children}</>;
  const cameIn = (said: string[] | null) => {
    setWithWord(false);
    if (said && said.length > 0) setCodes(said);
    else ask();
  };
  return (
    <Alone>
      {!standing.claimed ? (
        <Register standing={standing} first withACode={false} onIn={cameIn} />
      ) : who?.by === "code" ? (
        <Register standing={standing} first={false} withACode onIn={cameIn} />
      ) : withWord ? (
        <Register standing={standing} first={false} withACode={false} onIn={cameIn} onBack={() => setWithWord(false)} />
      ) : (
        <SignIn standing={standing} onIn={ask} onWord={() => setWithWord(true)} />
      )}
    </Alone>
  );
}
