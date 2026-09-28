// Setting containers up on this machine: what would be done is read first,
// agreed to as it was offered, then watched step by step.

import { Dialog } from "@base-ui/react/dialog";
import { useEffect, useState } from "react";
import { useStore } from "zustand";

import { providers, providing, type MachineWanted, type SetUp, type SetUpStep } from "./providers.ts";

const USUAL: MachineWanted = { cpus: 2, memory_mib: 2048, disk_gib: 20 };

const TONE: Record<SetUpStep["state"], string> = { waiting: "", running: " k-busy", done: " k-ready", failed: " k-danger" };
const WORD: Record<SetUpStep["state"], string> = { waiting: "", running: "Going on…", done: "Done", failed: "Failed" };

function Steps({ run }: { run: SetUp }) {
  const offered = run.state === "offered";
  return (
    <ol className="w-steps">
      {run.steps.map((step) => (
        <li className="w-step" key={step.command}>
          <span className={`k-dot k-small${TONE[step.state]}`} />
          <span className="k-name">
            {step.what}
            {offered || step.state === "waiting" ? null : <span className="k-caption"> · {WORD[step.state]}</span>}
          </span>
          <code className="k-mono">{step.command}</code>
          {step.said && step.state === "failed" ? <pre className="k-mono w-said">{step.said}</pre> : null}
        </li>
      ))}
    </ol>
  );
}

/// The machine's size, asked only when a machine is made.
function Size({ wanted, onChange }: { wanted: MachineWanted; onChange: (wanted: MachineWanted) => void }) {
  const whole = (typed: string, least: number) => Math.max(least, Math.round(Number(typed) || least));
  return (
    <div className="w-three">
      <label className="k-stack w-close">
        <span className="k-caption">Processors</span>
        <input className="k-field" type="number" min={1} value={wanted.cpus} onChange={(event) => onChange({ ...wanted, cpus: whole(event.target.value, 1) })} />
      </label>
      <label className="k-stack w-close">
        <span className="k-caption">Memory, GB</span>
        <input
          className="k-field"
          type="number"
          min={1}
          value={Math.round(wanted.memory_mib / 1024)}
          onChange={(event) => onChange({ ...wanted, memory_mib: whole(event.target.value, 1) * 1024 })}
        />
      </label>
      <label className="k-stack w-close">
        <span className="k-caption">Disk, up to GB</span>
        <input className="k-field" type="number" min={5} value={wanted.disk_gib} onChange={(event) => onChange({ ...wanted, disk_gib: whole(event.target.value, 1) })} />
      </label>
    </div>
  );
}

export function ContainersSetUp({ open, onClose }: { open: boolean; onClose: () => void }) {
  const run = useStore(providers, (state) => state.settingUp);
  const [wanted, setWanted] = useState(USUAL);
  const [reading, setReading] = useState(false);
  const [problem, setProblem] = useState("");

  const plan = async (size: MachineWanted) => {
    setReading(true);
    setProblem("");
    try {
      await providing.planContainers(size);
    } catch (error) {
      setProblem((error as Error).message);
    } finally {
      setReading(false);
    }
  };
  // Opened: what would be done is read, unless something is being done.
  useEffect(() => {
    if (open && providers.getState().settingUp?.state !== "running") void plan(wanted);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);
  // The size changed: the plan is read again once the typing stops.
  useEffect(() => {
    if (!open || run?.state !== "offered") return undefined;
    const soon = setTimeout(() => void plan(wanted), 500);
    return () => clearTimeout(soon);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wanted]);

  const agree = async () => {
    if (!run) return;
    setProblem("");
    try {
      await providing.setContainersUp(run.plan_id);
    } catch (error) {
      setProblem((error as Error).message);
    }
  };
  const offered = run?.state === "offered";
  const makes = run?.steps.some((step) => step.command.includes(" machine init ")) ?? false;
  const nothing = offered && run.steps.length === 0;

  return (
    <Dialog.Root open={open} onOpenChange={(next) => (next ? undefined : onClose())}>
      <Dialog.Portal>
        <Dialog.Backdrop className="w-scrim" />
        <Dialog.Popup className="k-dialog w-dialog w-wide">
          <Dialog.Title className="k-heading">Set up containers on this computer</Dialog.Title>
          <Dialog.Description className="k-caption">
            An agent in a container works in a Linux machine of its own and sees only its own folder. Podman runs that machine; here is what setting it up does.
          </Dialog.Description>
          {run === null ? <span className="k-caption">{reading ? "Reading what it takes…" : ""}</span> : null}
          {run && offered && makes ? <Size wanted={wanted} onChange={setWanted} /> : null}
          {run && offered && run.effects.length > 0 ? (
            <ul className="w-list">
              {run.effects.map((effect) => (
                <li key={effect}>{effect}</li>
              ))}
            </ul>
          ) : null}
          {nothing ? <div className="k-notice k-success">The machine is there and running. Look again to see it.</div> : null}
          {run ? <Steps run={run} /> : null}
          {run?.state === "running" ? <span className="k-caption">This takes a few minutes the first time. You can close this and go on working; it goes on.</span> : null}
          {run && offered
            ? run.blockers.map((blocker) => (
                <div className="k-notice k-danger" key={blocker}>
                  {blocker.replace(/^./, (first) => first.toUpperCase())}
                </div>
              ))
            : null}
          {run?.state === "done" ? <div className="k-notice k-success">{run.said}</div> : null}
          {run?.state === "failed" ? (
            <div className="k-notice k-danger w-col w-start-items">
              <span>Containers cannot be started yet.</span>
              {run.steps.some((step) => step.state === "failed") ? null : <pre className="k-mono w-said">{run.said}</pre>}
            </div>
          ) : null}
          {run && run.state !== "done" && run.hint ? (
            <div className="k-notice k-info w-col w-start-items">
              <span>{run.hint}</span>
              <a className="k-btn k-quiet" href="https://podman.io/docs/installation" target="_blank" rel="noreferrer">
                Get Podman's installer
              </a>
            </div>
          ) : null}
          {problem ? <div className="k-notice k-danger">{problem}</div> : null}
          <div className="k-inline w-end">
            <Dialog.Close className="k-btn k-quiet" type="button">
              {offered ? "Not now" : "Close"}
            </Dialog.Close>
            {run?.state === "failed" || (offered && run.blockers.length > 0) ? (
              <button type="button" className="k-btn k-primary" disabled={reading} onClick={() => void plan(wanted)}>
                {reading ? "Reading…" : "Try again"}
              </button>
            ) : null}
            {offered && !nothing && run.blockers.length === 0 ? (
              <button type="button" className="k-btn k-primary" disabled={reading} onClick={() => void agree()}>
                Set up
              </button>
            ) : null}
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
