// What the project is made of, and the two things a person does about it:
// add a slot, remove one. Each is one call of a tool the Cycle publishes.

import { useRef, useState } from "react";

import { callProjectTool } from "./api";
import type { SlotsView } from "./types";

interface Props {
  serverName: string;
  slots: SlotsView;
  onChanged: () => void;
}

export function Slots({ serverName, slots, onChanged }: Props) {
  const name = useRef<HTMLInputElement>(null);
  const module = useRef<HTMLSelectElement>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const act = async (tool: string, args: Record<string, unknown>) => {
    setError("");
    setBusy(true);
    try {
      await callProjectTool(serverName, tool, args);
      onChanged();
      if (name.current) name.current.value = "";
    } catch (failure) {
      setError((failure as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <section id="project-slots" aria-label="Slots">
      <h3>Slots</h3>
      {slots.slots.length === 0 ? (
        <p id="slots-empty" className="k-muted">
          Nothing yet. A project is made of slots - a named place for each kind of work; add the first one below.
        </p>
      ) : null}
      <div className="rows">
        {slots.slots.map((slot) => (
          <div className="slot-row k-row" data-slot={slot.slot} data-module={slot.module} key={slot.slot}>
            <strong>{slot.slot}</strong>
            <span className="k-muted">
              {slot.title || slot.module}
              {slot.app_uri ? "" : " · not in this build"}
              {slot.requires && slot.requires.length > 0 ? ` · needs ${slot.requires.join(", ")}` : ""}
            </span>
            <button data-remove-slot={slot.slot} disabled={busy} onClick={() => void act("remove_slot", { slot: slot.slot })}>
              Remove
            </button>
          </div>
        ))}
      </div>
      {slots.kinds.length > 0 ? (
        <form
          id="new-slot"
          className="new-slot"
          onSubmit={(event) => {
            event.preventDefault();
            const slot = name.current?.value.trim() ?? "";
            const kind = module.current?.value ?? "";
            if (slot && kind) void act("add_slot", { slot, module: kind });
          }}
        >
          <input id="new-slot-name" ref={name} placeholder="what to call it (music, site, place)" aria-label="Slot name" />
          <select id="new-slot-module" ref={module} aria-label="Kind of work" defaultValue={slots.kinds[0]?.module}>
            {slots.kinds.map((kind) => (
              <option value={kind.module} key={kind.module} title={kind.summary}>
                {kind.title || kind.module}
              </option>
            ))}
          </select>
          <button id="add-slot" type="submit" disabled={busy}>
            Add slot
          </button>
        </form>
      ) : null}
      <div id="slots-error" className="bad">
        {error}
      </div>
    </section>
  );
}
