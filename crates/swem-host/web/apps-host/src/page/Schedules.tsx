// An agent's schedules: what it is told on time, where, who made it, and
// how the last runs went.

import { Dialog } from "@base-ui/react/dialog";
import { useEffect, useState } from "react";
import { useForm } from "react-hook-form";
import { useStore } from "zustand";

import { Clock, Plus } from "./icons.tsx";
import { go } from "./place.ts";
import { HOW_A_RUN_ENDED, KEEPER_NAMES, asAField, chosenOf, keeperOf, ran, time, timing, until, whenOf, type Chosen, type Schedule } from "./time.ts";
import type { Participant } from "./types.ts";
import { chatsOf, world } from "./world.ts";

const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

interface Made extends Chosen {
  say: string;
  chat: string;
}

const USUAL: Chosen = { how: "daily", every: 30, unit: "minutes", at: "09:00", day: "1", once: "", line: "0 9 * * 1-5" };

/// A schedule made, or one that is there changed: what is said and when.
function ScheduleForm({ agent, open, changing, onClose }: { agent: Participant; open: boolean; changing: Schedule | null; onClose: () => void }) {
  const zone = useStore(time, (known) => known.keeper?.zone ?? "UTC");
  const order = useStore(world, (state) => chatsOf(state, agent.participant_id).map((chat) => chat.chat_id).join(" "));
  const known = useStore(world, (state) => state.chats);
  const chats = order.split(" ").filter(Boolean).flatMap((id) => (known[id] ? [known[id]] : []));
  const [problem, setProblem] = useState("");
  const usual = { ...USUAL, once: asAField(Date.now() + 60 * 60_000) };
  const form = useForm<Made>({
    values: changing
      ? { say: changing.say, chat: changing.chat_id ?? "", ...chosenOf(changing.when, usual) }
      : { say: "", chat: chats[0]?.chat_id ?? "", ...usual },
    resetOptions: { keepDirtyValues: true },
  });
  const how = form.watch("how");
  const close = () => {
    form.reset();
    setProblem("");
    onClose();
  };
  const make = form.handleSubmit(async (made) => {
    setProblem("");
    try {
      const when = whenOf(made, changing?.when.kind === "cron" ? changing.when.zone : zone);
      if (changing) await timing.change(agent.participant_id, changing.schedule_id, made.say, when);
      else await timing.make(agent.participant_id, made.say, when, made.chat || null);
      close();
    } catch (error) {
      setProblem((error as Error).message);
    }
  });
  return (
    <Dialog.Root open={open} onOpenChange={(next) => (next ? undefined : close())}>
      <Dialog.Portal>
        <Dialog.Backdrop className="w-scrim" />
        <Dialog.Popup className="k-dialog w-dialog">
          <Dialog.Title className="k-heading">{changing ? "Change the schedule" : "New schedule"}</Dialog.Title>
          <Dialog.Description className="k-caption">
            What {agent.name} is told, when, and where. It is said by the schedule, and answered like anything else said there.
          </Dialog.Description>
          <form className="k-stack" onSubmit={(event) => void make(event)}>
            <label className="k-stack w-close">
              <span className="k-caption">What {agent.name} is told</span>
              <textarea className="k-field" rows={3} {...form.register("say", { required: true })} />
            </label>
            <label className="k-stack w-close">
              <span className="k-caption">When</span>
              <select className="k-field" {...form.register("how")}>
                <option value="daily">Every day at a time</option>
                <option value="weekly">Every week on a day</option>
                <option value="every">Every so often</option>
                <option value="once">Once</option>
                <option value="cron">By a cron line</option>
              </select>
            </label>
            {how === "every" ? (
              <div className="w-pair">
                <label className="k-stack w-close">
                  <span className="k-caption">Every</span>
                  <input className="k-field" type="number" min={1} {...form.register("every", { valueAsNumber: true, min: 1 })} />
                </label>
                <label className="k-stack w-close">
                  <span className="k-caption">Of</span>
                  <select className="k-field" {...form.register("unit")}>
                    <option value="minutes">minutes</option>
                    <option value="hours">hours</option>
                    <option value="days">days</option>
                  </select>
                </label>
              </div>
            ) : null}
            {how === "weekly" ? (
              <label className="k-stack w-close">
                <span className="k-caption">On</span>
                <select className="k-field" {...form.register("day")}>
                  {DAYS.map((day, index) => (
                    <option value={String(index)} key={day}>
                      {day}
                    </option>
                  ))}
                </select>
              </label>
            ) : null}
            {how === "daily" || how === "weekly" ? (
              <label className="k-stack w-close">
                <span className="k-caption">At, {zone} time</span>
                <input className="k-field" type="time" {...form.register("at", { required: true })} />
              </label>
            ) : null}
            {how === "once" ? (
              <label className="k-stack w-close">
                <span className="k-caption">At</span>
                <input className="k-field" type="datetime-local" {...form.register("once", { required: true })} />
              </label>
            ) : null}
            {how === "cron" ? (
              <label className="k-stack w-close">
                <span className="k-caption">Minute, hour, day, month, day of the week; {zone} time</span>
                <input className="k-field k-mono" spellCheck={false} {...form.register("line", { required: true })} />
              </label>
            ) : null}
{changing ? (
              <span className="k-caption">Said in {changing.chat_id ? changing.chat_title || "its chat" : "a new chat each time"}.</span>
            ) : (
                          <label className="k-stack w-close">
                <span className="k-caption">Said in</span>
                <select className="k-field" {...form.register("chat")}>
                  {chats.map((chat) => (
                    <option value={chat.chat_id} key={chat.chat_id}>
                      {chat.title || "Its chat"}
                    </option>
                  ))}
                  <option value="">A new chat each time</option>
                </select>
              </label>
            )}
            {problem ? <div className="k-notice k-danger">{problem}</div> : null}
            <div className="k-inline w-end">
              <Dialog.Close className="k-btn k-quiet" type="button">
                Not now
              </Dialog.Close>
              <button type="submit" className="k-btn k-primary" disabled={form.formState.isSubmitting}>
                {changing ? "Save" : "Make the schedule"}
              </button>
            </div>
          </form>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function Row({ agent, schedule, now, onProblem, onChange }: { agent: Participant; schedule: Schedule; now: number; onProblem: (said: string) => void; onChange: () => void }) {
  const [busy, setBusy] = useState(false);
  const act = (what: Promise<void>) => {
    setBusy(true);
    onProblem("");
    what.catch((error: Error) => onProblem(error.message)).finally(() => setBusy(false));
  };
  return (
    <tr>
      <td>
        <span className="k-name">{schedule.say}</span>
      </td>
      <td>{schedule.when_in_words}</td>
      <td>
        {schedule.chat_id ? (
          <a
            href={`#/agents/${encodeURIComponent(agent.participant_id)}/chats/${encodeURIComponent(schedule.chat_id)}`}
            onClick={(event) => {
              event.preventDefault();
              go({ at: "agent", agent: agent.participant_id, tab: "chat", chat: schedule.chat_id ?? undefined });
            }}
          >
            {schedule.chat_title || "Its chat"}
          </a>
        ) : (
          "A new chat each time"
        )}
      </td>
      <td>{schedule.made_by === agent.participant_id ? agent.name : schedule.made_by_name}</td>
      <td className="k-muted">{!schedule.enabled ? (schedule.when.kind === "once" ? "said" : "paused") : schedule.next_due_ms ? until(schedule.next_due_ms, now) : ""}</td>
      <td className="w-end">
        <span className="k-inline w-tight w-end w-nowrap">
          <button
            type="button"
            className="k-switch"
            role="switch"
            aria-checked={schedule.enabled}
            aria-label={schedule.say}
            disabled={busy}
            onClick={() => act(timing.turn(agent.participant_id, schedule.schedule_id, !schedule.enabled))}
          >
            <span />
          </button>
          <button type="button" className="k-btn k-quiet" disabled={busy} onClick={onChange}>
            Change
          </button>
          <button type="button" className="k-btn k-quiet" disabled={busy} onClick={() => act(timing.forget(agent.participant_id, schedule.schedule_id))}>
            Forget
          </button>
        </span>
      </td>
    </tr>
  );
}

export function Schedules({ agent, hidden }: { agent: Participant; hidden: boolean }) {
  const kept = useStore(time, (known) => known.of[agent.participant_id]);
  const keeper = useStore(time, (known) => known.keeper);
  const mine = keeperOf(keeper, agent.profile_id);
  const [making, setMaking] = useState(false);
  const [changing, setChanging] = useState<Schedule | null>(null);
  const [problem, setProblem] = useState("");
  const [now, setNow] = useState(Date.now());
  // What is due and how it went changes by itself: it is read again while
  // somebody looks.
  useEffect(() => {
    if (hidden) return undefined;
    const read = () => {
      setNow(Date.now());
      timing.read(agent.participant_id).catch((error: Error) => setProblem(error.message));
    };
    read();
    void timing.keeper().catch(() => {});
    const again = window.setInterval(read, 8000);
    return () => window.clearInterval(again);
  }, [hidden, agent.participant_id]);
  return (
    <section className="w-page" data-agent-panel="schedules" hidden={hidden} aria-label={`Schedules of ${agent.name}`}>
      <section className="k-card k-spread">
        <div className="k-inline w-nowrap">
          <Clock size={20} />
          <div className="w-col w-close">
            <span className="k-title">{mine === null ? "Time" : `${agent.name}'s time is kept by ${mine.id === "swem" ? "this SWEM" : "the system's scheduler"}`}</span>
            <span className="k-caption">
              {agent.name}'s machine is not kept awake to watch the clock. When something is due, SWEM hands over the message.{" "}
              {mine?.id === "system"
                ? "While the Workbench is closed this computer starts SWEM to say it."
                : "Nothing is said while SWEM is closed; what was missed is said once when it is back."}
              {keeper?.keepers.some((one) => one.id === "outside" && one.on) ? " A scheduler outside knocks on this SWEM as well." : ""}
            </span>
          </div>
        </div>
        <div className="k-inline w-tight w-nowrap">
          {keeper && agent.profile_id ? (
            <select
              className="k-field"
              aria-label={`Who keeps ${agent.name}'s time`}
              value={keeper.chosen[agent.profile_id] ?? ""}
              onChange={(event) => {
                setProblem("");
                timing.choose(agent.profile_id ?? "", event.target.value || null).catch((error: Error) => setProblem(error.message));
              }}
            >
              <option value="">{`As agents have it: ${KEEPER_NAMES[keeper.keepers.find((one) => one.default)?.id ?? "swem"]}`}</option>
              {keeper.keepers
                .filter((one) => one.id !== "outside")
                .map((one) => (
                  <option value={one.id} key={one.id} disabled={one.id === "system" && !one.on}>
                    {one.id === "system" && !one.on ? `${KEEPER_NAMES[one.id]} (off)` : KEEPER_NAMES[one.id]}
                  </option>
                ))}
            </select>
          ) : null}
          <button type="button" className="k-btn" onClick={() => go({ at: "providers", tab: "time" })}>
            Who keeps time
          </button>
        </div>
      </section>
      <section className="k-card k-stack">
        <div className="k-spread">
          <div className="w-col w-close">
            <h2 className="k-heading">Schedules</h2>
            <span className="k-caption">
              A schedule is a message that arrives on time. {agent.name} can make one itself when you ask it to remind or to check back.
            </span>
          </div>
          <button type="button" className="k-btn k-primary" onClick={() => setMaking(true)}>
            <Plus size={15} />
            <span>New schedule</span>
          </button>
        </div>
        {kept === undefined ? (
          <span className="k-caption">Reading…</span>
        ) : kept.schedules.length === 0 ? (
          <span className="k-caption">Nothing is said to {agent.name} on time yet.</span>
        ) : (
          <table className="w-table">
            <thead>
              <tr>
                <th scope="col" className="k-eyebrow">What {agent.name} is told</th>
                <th scope="col" className="k-eyebrow">When</th>
                <th scope="col" className="k-eyebrow">Said in</th>
                <th scope="col" className="k-eyebrow">Made by</th>
                <th scope="col" className="k-eyebrow">Next</th>
                <th scope="col" className="k-eyebrow w-end">On</th>
              </tr>
            </thead>
            <tbody>
              {kept.schedules.map((schedule) => (
                <Row agent={agent} schedule={schedule} now={now} onProblem={setProblem} onChange={() => setChanging(schedule)} key={schedule.schedule_id} />
              ))}
            </tbody>
          </table>
        )}
        {problem ? <div className="k-notice k-danger">{problem}</div> : null}
      </section>
      {kept && kept.runs.length > 0 ? (
        <section className="k-card k-stack">
          <h2 className="k-heading">Last runs</h2>
          <div>
            {kept.runs.map((run) => {
              const ended = HOW_A_RUN_ENDED[run.state];
              return (
                <div className="k-row w-nowrap" key={`${run.schedule_id} ${run.due_ms}`}>
                  <span className="k-grow w-one-line">{run.say}</span>
                  <span className="k-caption">{ran(run, now)}</span>
                  <span className="k-status">
                    <span className={`k-dot k-small${ended.tone === "none" ? "" : ` k-${ended.tone}`}`} />
                    <span>{run.late && run.state === "answered" ? "Ran late" : ended.words}</span>
                  </span>
                </div>
              );
            })}
          </div>
        </section>
      ) : null}
      <ScheduleForm
        agent={agent}
        open={making || changing !== null}
        changing={changing}
        onClose={() => {
          setMaking(false);
          setChanging(null);
        }}
      />
    </section>
  );
}
