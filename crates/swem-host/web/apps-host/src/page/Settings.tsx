// Settings: this Workbench, where it is and who may come in.

import { Dialog } from "@base-ui/react/dialog";
import { useCallback, useEffect, useState, type FormEvent, type ReactNode } from "react";

import { Codes } from "./Door.tsx";
import { accessShown, doorStanding, makeToken, mayInWords, newCodes, signOut, takeAway, withdraw, wordForADevice, type AccessShown, type Device, type May, type Token } from "./door.ts";
import { Globe, Laptop, Lock, Phone, Plug, Plus } from "./icons.tsx";
import type { SettingsTab } from "./place.ts";
import { when } from "./providers.ts";
import { Standing } from "./standing.tsx";

const told = (error: unknown): string => (error instanceof Error ? error.message : String(error));

function Part({ title, about, action, children }: { title: string; about: string; action?: ReactNode; children?: ReactNode }) {
  return (
    <section className="k-card k-stack" aria-label={title}>
      <div className="k-spread w-nowrap w-top">
        <div className="w-col w-close">
          <h2 className="k-heading">{title}</h2>
          <span className="k-caption">{about}</span>
        </div>
        {action}
      </div>
      {children}
    </section>
  );
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="w-fact">
      <span className="k-caption">{label}</span>
      <span className="k-name">{children}</span>
    </div>
  );
}

const WAYS: Record<AccessShown["way"], string> = {
  certificate: "Closed, by its own certificate",
  proxy: "Closed, by the proxy in front of it",
  "this-machine": "On this computer alone",
};

const last = (at: number | null, from: string | null): string => (at === null ? "Never" : [when(at), from].filter(Boolean).join(" · "));

/// Something said once, in a dialog that is closed by whoever read it.
function SaidOnce({ title, about, onClose, children }: { title: string; about: string; onClose: () => void; children: ReactNode }) {
  return (
    <Dialog.Root open onOpenChange={(next) => (next ? undefined : onClose())}>
      <Dialog.Portal>
        <Dialog.Backdrop className="w-scrim" />
        <Dialog.Popup className="k-dialog w-dialog">
          <Dialog.Title className="k-heading">{title}</Dialog.Title>
          <Dialog.Description className="k-caption">{about}</Dialog.Description>
          {children}
          <div className="k-inline w-end">
            <Dialog.Close className="k-btn k-primary" type="button">
              Done
            </Dialog.Close>
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function Asked({ title, about, yes, onAnswer }: { title: string; about: string; yes: string; onAnswer: (yes: boolean) => void }) {
  return (
    <Dialog.Root open onOpenChange={(next) => (next ? undefined : onAnswer(false))}>
      <Dialog.Portal>
        <Dialog.Backdrop className="w-scrim" />
        <Dialog.Popup className="k-dialog w-dialog">
          <Dialog.Title className="k-heading">{title}</Dialog.Title>
          <Dialog.Description className="k-caption">{about}</Dialog.Description>
          <div className="k-inline w-end w-tight">
            <button type="button" className="k-btn k-quiet" onClick={() => onAnswer(false)}>
              Not now
            </button>
            <button type="button" className="k-btn k-primary" onClick={() => onAnswer(true)}>
              {yes}
            </button>
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function NewToken({ onClose, onMade }: { onClose: () => void; onMade: (opens: string, token: Token) => void }) {
  const [name, setName] = useState("");
  const [may, setMay] = useState<May>("say-what-is-due");
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    try {
      const made = await makeToken(name, [may]);
      onMade(made.opens, made.token);
    } catch (error) {
      setProblem(told(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog.Root open onOpenChange={(next) => (next ? undefined : onClose())}>
      <Dialog.Portal>
        <Dialog.Backdrop className="w-scrim" />
        <Dialog.Popup className="k-dialog w-dialog">
          <Dialog.Title className="k-heading">Make a token</Dialog.Title>
          <Dialog.Description className="k-caption">A program comes in with a token. What opens with it is shown once, when it is made.</Dialog.Description>
          <form className="w-col" onSubmit={(event) => void submit(event)}>
            <label className="w-col w-close">
              <span className="k-caption">What it is for</span>
              <input className="k-field" value={name} onChange={(event) => setName(event.target.value)} maxLength={60} placeholder="Morning scheduler" required autoFocus />
            </label>
            <fieldset className="w-fieldset">
              <legend className="k-caption">What it may do</legend>
              <label className="k-inline w-tight w-nowrap">
                <input type="radio" name="may" checked={may === "say-what-is-due"} onChange={() => setMay("say-what-is-due")} />
                <span>Say that something is due, and nothing else</span>
              </label>
              <label className="k-inline w-tight w-nowrap">
                <input type="radio" name="may" checked={may === "everything"} onChange={() => setMay("everything")} />
                <span>What the page does: talk to agents, read their files, open their terminals</span>
              </label>
            </fieldset>
            {problem ? (
              <div className="k-notice k-danger" role="alert">
                {problem}
              </div>
            ) : null}
            <div className="k-inline w-end w-tight">
              <button type="button" className="k-btn k-quiet" onClick={onClose}>
                Not now
              </button>
              <button type="submit" className="k-btn k-primary" disabled={busy || !name.trim()}>
                Make it
              </button>
            </div>
          </form>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function CopyIt({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      className="k-btn"
      onClick={() => {
        void navigator.clipboard?.writeText(text).then(() => {
          setCopied(true);
          window.setTimeout(() => setCopied(false), 1600);
        });
      }}
    >
      {copied ? "Copied" : "Copy"}
    </button>
  );
}

type Open =
  | { what: "word"; word: string; minutes: number }
  | { what: "new-token" }
  | { what: "token"; opens: string; name: string }
  | { what: "codes-asked" }
  | { what: "codes"; codes: string[] }
  | { what: "take-away"; device: Device }
  | { what: "withdraw"; token: Token }
  | null;

function Served({ shown, again }: { shown: AccessShown; again: () => void }) {
  const [open, setOpen] = useState<Open>(null);
  const [problem, setProblem] = useState("");
  const doing = async (what: () => Promise<void>) => {
    setProblem("");
    try {
      await what();
    } catch (error) {
      setProblem(told(error));
    }
    again();
  };
  return (
    <>
      {problem ? (
        <div className="k-notice k-danger" role="alert">
          {problem}
        </div>
      ) : null}
      <div className="w-columns">
        <div className="w-col">
          <Part title="Where it is" about="What a browser opens, and how the way to it is kept closed." action={<Standing tone="ready">Served</Standing>}>
            <div className="w-facts">
              <Fact label="Address">{shown.at}</Fact>
              <Fact label="The way to it">{WAYS[shown.way]}</Fact>
              <Fact label="Apps are drawn at">{shown.apps_at ?? "Nowhere"}</Fact>
              {shown.certificate_good_until_ms === null ? null : (
                <Fact label="The certificate is good until">{new Date(shown.certificate_good_until_ms).toLocaleDateString([], { day: "numeric", month: "long", year: "numeric" })}</Fact>
              )}
            </div>
          </Part>
          <Part
            title="Devices that may come in"
            about="Each holds a key of its own. Take one away and it comes in no more."
            action={
              <button
                type="button"
                className="k-btn"
                onClick={() =>
                  void doing(async () => {
                    const made = await wordForADevice();
                    setOpen({ what: "word", word: made.word, minutes: made.good_for_minutes });
                  })
                }
              >
                <Plus />
                <span>Add a device</span>
              </button>
            }
          >
            <div className="w-col w-close">
              {shown.devices.map((device) => (
                <div className="k-row w-nowrap" key={device.device_id}>
                  {/phone|android/i.test(device.name) ? <Phone size={18} /> : <Laptop size={18} />}
                  <span className="w-col w-close k-grow">
                    <span className="k-name">{device.name}</span>
                    <span className="k-caption">Added {when(device.added_ms)}</span>
                  </span>
                  <span className="k-caption">{device.this ? "Here now" : last(device.last_ms, device.last_from)}</span>
                  {device.this ? (
                    <span className="k-chip">This device</span>
                  ) : (
                    <button type="button" className="k-btn k-quiet" onClick={() => setOpen({ what: "take-away", device })}>
                      Take away
                    </button>
                  )}
                </div>
              ))}
            </div>
          </Part>
          <Part
            title="Codes to come back with"
            about={
              shown.codes.made === 0
                ? "For when every device is lost. None were made."
                : `For when every device is lost. ${shown.codes.left} of ${shown.codes.made} are left${shown.codes.made_ms ? `, made ${when(shown.codes.made_ms)}` : ""}.`
            }
            action={
              <button type="button" className="k-btn" onClick={() => setOpen({ what: "codes-asked" })}>
                Make new ones
              </button>
            }
          />
        </div>
        <div className="w-col">
          <Part
            title="Tokens for programs"
            about="A program comes in with a token that says what it may do. It is shown once, when it is made."
            action={
              <button type="button" className="k-btn" onClick={() => setOpen({ what: "new-token" })}>
                <Plus />
                <span>Make a token</span>
              </button>
            }
          >
            {shown.tokens.length === 0 ? (
              <span className="k-caption">No program may come in.</span>
            ) : (
              <div className="w-col w-close">
                {shown.tokens.map((token) => (
                  <div className="k-row w-nowrap" key={token.token_id}>
                    {token.may.includes("everything") ? <Plug size={18} /> : <Globe size={18} />}
                    <span className="w-col w-close k-grow">
                      <span className="k-name">{token.name}</span>
                      <span className="k-caption">{mayInWords(token.may)}</span>
                    </span>
                    <span className="k-caption">{token.last_ms === null ? "Never used" : last(token.last_ms, token.last_from)}</span>
                    <button type="button" className="k-btn k-quiet" onClick={() => setOpen({ what: "withdraw", token })}>
                      Withdraw
                    </button>
                  </div>
                ))}
              </div>
            )}
          </Part>
          <Part title="What was done" about="Who came in, by what, from where, and what was refused.">
            <div className="w-col w-close">
              {shown.happened.map((one, index) => (
                <div className="w-happened" key={`${one.at_ms}-${index}`}>
                  <span className={`k-dot k-small ${one.refused ? "k-danger" : "k-ready"}`} />
                  <span className="k-grow">{one.what}</span>
                  <span className="k-caption">{[one.who, one.from].filter(Boolean).join(" · ")}</span>
                  <span className="k-caption w-end-text">{when(one.at_ms)}</span>
                </div>
              ))}
            </div>
          </Part>
        </div>
      </div>
      {open?.what === "word" ? (
        <SaidOnce title="A word for one more device" about={`Open ${shown.at} on the device, choose to register it with a word, and give it this one. It is used once and is good for ${open.minutes} minutes.`} onClose={() => setOpen(null)}>
          <div className="w-said-once">{open.word}</div>
        </SaidOnce>
      ) : null}
      {open?.what === "new-token" ? (
        <NewToken
          onClose={() => setOpen(null)}
          onMade={(opens, token) => {
            setOpen({ what: "token", opens, name: token.name });
            again();
          }}
        />
      ) : null}
      {open?.what === "token" ? (
        <SaidOnce title={`The token of ${open.name}`} about="Give it to the program now. The Workbench keeps it only in a form it cannot read back and cannot show it again." onClose={() => setOpen(null)}>
          <div className="w-said-once">{open.opens}</div>
          <div className="k-inline">
            <CopyIt text={open.opens} />
          </div>
        </SaidOnce>
      ) : null}
      {open?.what === "codes-asked" ? (
        <Asked
          title="Make new codes"
          about="The codes made before are good no more. The new ones are shown once."
          yes="Make new ones"
          onAnswer={(yes) => {
            setOpen(null);
            if (yes)
              void doing(async () => {
                const made = await newCodes();
                setOpen({ what: "codes", codes: made.codes });
              });
          }}
        />
      ) : null}
      {open?.what === "codes" ? (
        <SaidOnce title="Codes to come back with" about="If every device of yours is lost, one of these lets you in to register a new one. Each is used once." onClose={() => setOpen(null)}>
          <Codes codes={open.codes} />
        </SaidOnce>
      ) : null}
      {open?.what === "take-away" ? (
        <Asked
          title={`Take ${open.device.name} away`}
          about="It comes in no more, and what it has open is closed."
          yes="Take it away"
          onAnswer={(yes) => {
            const device = open.device;
            setOpen(null);
            if (yes) void doing(async () => void (await takeAway(device.device_id)));
          }}
        />
      ) : null}
      {open?.what === "withdraw" ? (
        <Asked
          title={`Withdraw the token of ${open.token.name}`}
          about="The program that holds it comes in no more."
          yes="Withdraw"
          onAnswer={(yes) => {
            const token = open.token;
            setOpen(null);
            if (yes) void doing(async () => void (await withdraw(token.token_id)));
          }}
        />
      ) : null}
    </>
  );
}

function OnThisComputer() {
  return (
    <Part title="Where it is" about="What a browser opens, and how the way to it is kept closed." action={<Standing tone="ready">On this computer alone</Standing>}>
      <span>This Workbench listens on this computer and nowhere else. It opens at the address it printed when it started, which carries the secret of that run.</span>
      <span className="k-caption">To come to it from a laptop and a phone, start it on a server at a name. Whoever comes then signs in with a passkey, and programs come in with tokens made here.</span>
      <div className="w-said-once">swem workbench serve --at https://workbench.example.org --tls-cert chain.pem --tls-key key.pem</div>
      <span className="k-caption">With a proxy of yours in front, which holds the certificate:</span>
      <div className="w-said-once">swem workbench serve --at https://workbench.example.org --behind-proxy --apps-at https://apps.workbench.example.org</div>
    </Part>
  );
}

function Access() {
  const [shown, setShown] = useState<AccessShown | null>(null);
  const [here, setHere] = useState(false);
  const [problem, setProblem] = useState("");
  const again = useCallback(() => {
    doorStanding()
      .then((door) => {
        if (door.at === null) {
          setHere(true);
          return undefined;
        }
        return accessShown().then(setShown);
      })
      .catch((error: unknown) => setProblem(told(error)));
  }, []);
  useEffect(again, [again]);
  if (problem) {
    return (
      <div className="k-notice k-danger" role="alert">
        {problem}
      </div>
    );
  }
  if (here) return <OnThisComputer />;
  return shown ? <Served shown={shown} again={again} /> : null;
}

export function Settings({ tab }: { tab: SettingsTab }) {
  const [served, setServed] = useState(false);
  useEffect(() => {
    doorStanding()
      .then((door) => setServed(door.at !== null))
      .catch(() => {});
  }, []);
  return (
    <main className="w-main">
      <header className="w-head">
        <div className="k-spread">
          <div className="w-col w-close">
            <h1 className="w-h1">Settings</h1>
            <span className="k-caption">This Workbench: where it is and who may come in.</span>
          </div>
          {served ? (
            <button
              type="button"
              className="k-btn k-quiet"
              onClick={() => {
                void signOut().finally(() => window.location.reload());
              }}
            >
              Sign out
            </button>
          ) : null}
        </div>
        <nav className="k-tabs" aria-label="Settings">
          <button type="button" className={`k-tab${tab === "access" ? " k-active" : ""}`} aria-current="page">
            <Lock size={15} />
            <span>Access</span>
          </button>
        </nav>
      </header>
      <div className="w-page">
        <Access />
      </div>
    </main>
  );
}
