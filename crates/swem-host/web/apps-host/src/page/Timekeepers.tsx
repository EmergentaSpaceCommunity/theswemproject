// Who keeps time. Time is never kept inside an agent's machine, so somebody
// outside it keeps it, and who that is differs by what it can do: this SWEM
// while it runs, the system's scheduler while the Workbench is closed, an
// outside scheduler for a SWEM that is reached from elsewhere.

import { Dialog } from "@base-ui/react/dialog";
import { useEffect, useState, type ReactNode } from "react";
import { useStore } from "zustand";

import { Clock, Laptop, Plug } from "./icons.tsx";
import { when } from "./providers.ts";
import { Standing, UsedBy } from "./standing.tsx";
import { KEEPER_NAMES, time, timing, type KeeperLook, type KeeperShown } from "./time.ts";

/// What a keeper that looks and leaves found the last time.
function looked(look: KeeperLook): string {
  const found = look.kept_elsewhere ? "the Workbench was keeping time" : look.said === 0 ? "nothing was due" : look.said === 1 ? "one message was said" : `${look.said} messages were said`;
  const said = look.said === 0 && look.said_last_ms ? ` It last said something at ${when(look.said_last_ms)}.` : "";
  return `Looked last at ${when(look.looked_ms)}: ${found}.${said}`;
}

function Row({ sign, keeper, about, limits, standing, children }: { sign: ReactNode; keeper: KeeperShown; about: string; limits: string; standing: ReactNode; children?: ReactNode }) {
  return (
    <div className="k-row w-nowrap w-top">
      {sign}
      <span className="k-two k-grow">
        <span className="k-inline w-tight">
          <span className="k-name">{KEEPER_NAMES[keeper.id]}</span>
          {keeper.default ? <span className="k-badge">Default</span> : null}
        </span>
        <span className="k-caption">{about}</span>
        <span className="k-caption">{limits}</span>
        {keeper.last_look ? <span className="k-caption">{looked(keeper.last_look)}</span> : null}
        {keeper.said ? <span className="k-caption k-is-danger">{keeper.said}</span> : null}
      </span>
      {keeper.id === "outside" ? null : <UsedBy profiles={keeper.used_by} />}
      <span className="w-col w-end-items">
        {standing}
        {children}
      </span>
    </div>
  );
}

/// Turning the system's scheduler on: what is put on this computer is
/// read first.
function TurnOn({ keeper, onClose }: { keeper: KeeperShown | null; onClose: () => void }) {
  const [forAll, setForAll] = useState(true);
  const [turning, setTurning] = useState(false);
  const [problem, setProblem] = useState("");
  const turn = async () => {
    setTurning(true);
    setProblem("");
    try {
      await timing.turnSystemOn(forAll);
      onClose();
    } catch (error) {
      setProblem((error as Error).message);
    } finally {
      setTurning(false);
    }
  };
  return (
    <Dialog.Root open={keeper !== null} onOpenChange={(next) => (next ? undefined : onClose())}>
      <Dialog.Portal>
        <Dialog.Backdrop className="w-scrim" />
        <Dialog.Popup className="k-dialog w-dialog">
          <Dialog.Title className="k-heading">Let this computer keep time</Dialog.Title>
          <Dialog.Description className="k-caption">
            Schedules are said also when the Workbench is closed. This is what it takes.
          </Dialog.Description>
          <ul className="w-list">
            <li>A job is given to {keeper?.called || "the system's scheduler"}. Every minute it starts SWEM, which looks at what is due, says it, waits for the answer and leaves.</li>
            <li>While the Workbench is open it keeps time itself, and the job leaves at once.</li>
            <li>The job holds the command and where your data is. It holds no key.</li>
            <li>An agent that works on this computer is started on it while you do other things.</li>
          </ul>
          <label className="k-inline w-tight w-nowrap w-top">
            <input type="checkbox" checked={forAll} onChange={(event) => setForAll(event.target.checked)} />
            <span>Keep time this way for every agent that was given no other keeper</span>
          </label>
          {problem ? <div className="k-notice k-danger">{problem}</div> : null}
          <div className="k-inline w-end">
            <Dialog.Close className="k-btn k-quiet" type="button">
              Not now
            </Dialog.Close>
            <button type="button" className="k-btn k-primary" disabled={turning} onClick={() => void turn()}>
              {turning ? "Turning on…" : "Turn on"}
            </button>
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

export function Timekeepers() {
  const keeper = useStore(time, (known) => known.keeper);
  const [problem, setProblem] = useState("");
  const [turningOn, setTurningOn] = useState<KeeperShown | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    const read = () => timing.keeper().catch((error: Error) => setProblem(error.message));
    void read();
    // When a keeper looked last changes by itself.
    const again = window.setInterval(read, 15000);
    return () => window.clearInterval(again);
  }, []);
  const doing = async (what: () => Promise<void>) => {
    setBusy(true);
    setProblem("");
    try {
      await what();
    } catch (error) {
      setProblem((error as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const by = (id: KeeperShown["id"]) => keeper?.keepers.find((one) => one.id === id) ?? null;
  const swem = by("swem");
  const system = by("system");
  const outside = by("outside");
  return (
    <>
      <TurnOn keeper={turningOn} onClose={() => setTurningOn(null)} />
      <div className="k-notice">
        Time is never kept inside an agent's machine. A machine that sleeps between messages costs nothing between runs, and is woken only to do the work.
      </div>
      <section className="k-card k-stack">
        <div className="w-col w-close">
          <h2 className="k-heading">Who keeps time</h2>
          <span className="k-caption">
            A schedule is a message that arrives on time. Whoever keeps an agent's time stores it and, when it is due, has SWEM say it in its chat.
            {keeper ? ` A time of day is kept in ${keeper.zone} time.` : ""}
          </span>
        </div>
        {problem ? <div className="k-notice k-danger">{problem}</div> : null}
        {keeper === null ? <span className="k-caption">Reading…</span> : null}
        {swem ? (
          <Row
            sign={<Clock size={18} />}
            keeper={swem}
            about="Built in. Keeps time for as long as SWEM runs."
            limits="Nothing is said while SWEM is closed or this computer is asleep; what was due then is said once, late."
            standing={<Standing tone={swem.on ? "ready" : "asking"}>{swem.on ? "Keeping time" : "Another SWEM here keeps it now"}</Standing>}
          >
            {swem.default ? null : (
              <button type="button" className="k-btn k-quiet" disabled={busy} onClick={() => void doing(() => timing.makeDefault("swem"))}>
                Make default
              </button>
            )}
          </Row>
        ) : null}
        {system ? (
          <Row
            sign={<Laptop size={18} />}
            keeper={system}
            about={`Built in${system.called ? `, by ${system.called}` : ""}. This computer starts SWEM when something is due, also when the Workbench is closed.`}
            limits="Needs this computer to be on and you signed in to it. What was due while it was asleep is said once it wakes."
            standing={<Standing tone={system.on ? "ready" : "none"}>{!system.available ? "Not on this system yet" : system.on ? "On" : "Off"}</Standing>}
          >
            {!system.available ? null : system.on ? (
              <span className="k-inline w-tight">
                {system.default ? null : (
                  <button type="button" className="k-btn k-quiet" disabled={busy} onClick={() => void doing(() => timing.makeDefault("system"))}>
                    Make default
                  </button>
                )}
                <button type="button" className="k-btn k-quiet" disabled={busy} onClick={() => void doing(() => timing.turnSystemOff())}>
                  Turn off
                </button>
              </span>
            ) : (
              <button type="button" className="k-btn k-primary" disabled={busy} onClick={() => setTurningOn(system)}>
                Turn on
              </button>
            )}
          </Row>
        ) : null}
        {outside ? (
          <Row
            sign={<Plug size={18} />}
            keeper={outside}
            about="A service that calls SWEM on time, for a SWEM that runs where it can be reached. It is what wakes an agent whose machine sleeps elsewhere."
            limits="Needs an address and a key."
            standing={<Standing tone="none">Cannot be added yet</Standing>}
          />
        ) : null}
      </section>
    </>
  );
}
