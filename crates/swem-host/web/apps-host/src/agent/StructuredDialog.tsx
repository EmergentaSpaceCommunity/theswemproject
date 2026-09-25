// A structured interaction - an ACP elicitation, a URL consent, or a
// server-owned structured fallback - as the flat form `flatForm.ts` models.
//
// The inputs are the person's answers and are read when the form is
// answered, not mirrored keystroke by keystroke: what the form holds is what
// is sent, whichever way it got there.

import { useMemo, useRef, useState } from "react";

import { FieldInput, readRaw } from "./FlatFormFields.tsx";
import { fieldsOf, valueOf } from "./flatForm.ts";
import { sessionStore, useSession } from "./store.ts";

export function StructuredDialog() {
  const { structured, structuredError } = useSession();
  const form = useRef<HTMLFormElement>(null);
  const [shapeError, setShapeError] = useState("");
  const fields = useMemo(() => {
    if (!structured) return [];
    try {
      setShapeError("");
      return fieldsOf(structured.schema);
    } catch (error) {
      // A shape the flat form cannot carry is refused where the person can
      // see it, never rendered as a text box that accepts the wrong thing.
      const message = (error as Error).message;
      setShapeError(message);
      sessionStore.setAppsStatus(`structured fallback unavailable: ${message}`);
      return [];
    }
  }, [structured]);
  const value = () => {
    if (form.current && !form.current.reportValidity()) throw new Error("form validation failed");
    return valueOf(fields, readRaw(form.current));
  };
  const phase = structured?.phase;
  const acceptLabel = phase === "launch" ? "Request input" : phase === "acp_url" ? "Consent and open" : "Accept";
  // A new interaction gets fresh inputs; the key is what makes that so.
  const generation = structured ? `${structured.phase}:${structured.sequence ?? ""}:${structured.interactionId ?? ""}:${structured.context}` : "";
  return (
    <div id="elicitation" style={{ display: structured ? "grid" : "none" }}>
      <form
        className="elicitation-card k-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="The agent asks for details"
        ref={form}
        onSubmit={(event) => {
          event.preventDefault();
          void sessionStore.answerStructured("accept", value);
        }}
      >
        <div className="k-eyebrow">The agent asks for details</div>
        <div id="elicitation-context">{structured?.context ?? ""}</div>
        <div id="elicitation-message">{structured?.message ?? ""}</div>
        <div id="elicitation-fields" key={generation}>
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
        </div>
        <div id="elicitation-error">{structuredError || shapeError}</div>
        <div className="elicitation-actions">
          <button id="elicitation-accept" className="primary" type="submit">
            {acceptLabel}
          </button>
          <button id="elicitation-decline" type="button" hidden={phase === "launch"} onClick={() => void sessionStore.answerStructured("decline", value)}>
            Decline
          </button>
          <button id="elicitation-cancel" type="button" onClick={() => void sessionStore.answerStructured("cancel", value)}>
            Cancel
          </button>
        </div>
      </form>
    </div>
  );
}
