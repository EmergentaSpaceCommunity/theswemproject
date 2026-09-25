// A step on the project as a form: the fields the tool's own schema asks
// for, minus what the rung already decided, sent as one call.

import { useMemo, useRef, useState } from "react";

import { FieldInput, readRaw } from "../agent/FlatFormFields.tsx";
import { fieldsOf, valueOf } from "../agent/flatForm.ts";
import { callProjectTool } from "./api";
import { formSchemaFor, undecidedStructuredArgument } from "./slots";
import type { RungAction, ToolView } from "./types";

interface Props {
  serverName: string;
  action: RungAction;
  tool: ToolView;
  onDone: () => void;
  onCancel: () => void;
}

/// Whether this tool can be a form at all; `null` names the reason it cannot.
export function formShapeProblem(tool: ToolView, action: RungAction): string | null {
  try {
    const schema = formSchemaFor(tool.input_schema, action.preset);
    const structured = undecidedStructuredArgument(schema);
    if (structured) return structured;
    fieldsOf(schema);
    return null;
  } catch (error) {
    return (error as Error).message;
  }
}

export function ToolForm({ serverName, action, tool, onDone, onCancel }: Props) {
  const form = useRef<HTMLFormElement>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const fields = useMemo(() => fieldsOf(formSchemaFor(tool.input_schema, action.preset)), [tool, action]);
  const submit = async () => {
    setError("");
    if (form.current && !form.current.reportValidity()) return;
    let value: Record<string, unknown>;
    try {
      value = { ...(action.preset ?? {}), ...valueOf(fields, readRaw(form.current)) };
    } catch (failure) {
      setError((failure as Error).message);
      return;
    }
    setBusy(true);
    try {
      await callProjectTool(serverName, action.tool, value);
      onDone();
    } catch (failure) {
      setError((failure as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <form
      id="rung-action-form"
      className="rung-form"
      data-action-tool={action.tool}
      ref={form}
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <div className="k-eyebrow">{action.label}</div>
      {tool.description ? <small className="k-muted">{tool.description}</small> : null}
      {fields.map((field) => (
        <label className="elicitation-field" key={field.name}>
          <span>
            {field.label}
            {field.required ? " *" : ""}
          </span>
          <FieldInput field={field} />
          {field.description ? <small>{field.description}</small> : null}
        </label>
      ))}
      <div id="rung-action-error" className="bad">
        {error}
      </div>
      <div className="elicitation-actions">
        <button id="rung-action-submit" className="primary" type="submit" disabled={busy}>
          {busy ? "Working…" : action.label}
        </button>
        <button id="rung-action-cancel" type="button" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
