// What a server lets a person ask of it with a form, and not the agent.
//
// A person fills in what the tool takes; the server answers with the form it
// needs filled in, and what the person answers there is what happens. The
// agent takes no part in it and is told nothing about it.

import { useMemo, useRef, useState } from "react";

import { FieldInput, readRaw } from "../agent/FlatFormFields.tsx";
import { fieldsOf, valueOf } from "../agent/flatForm.ts";
import type { AppTool } from "../agent/session.ts";
import { fetchJson } from "../http.ts";

const part = encodeURIComponent;
const post = (body: unknown): RequestInit => ({
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify(body),
});

/// What a tool is called when its server gives it no title: its name, as words.
export const toolNamed = (tool: AppTool): string => tool.title ?? tool.name.replace(/[-_]/g, " ");

/// The tools of a server that are a person's to call and not the agent's.
export const forAPerson = (tools: AppTool[], uri: string): AppTool[] =>
  tools.filter((tool) => tool.resource_uri === uri && tool.visibility.includes("app") && !tool.visibility.includes("model"));

interface Asked {
  interaction_id: string;
  message: string;
  requested_schema: unknown;
}

export function AppForm({
  connection,
  server,
  tool,
  onDone,
}: {
  connection: () => Promise<string | null>;
  server: string;
  tool: AppTool;
  /// The form is over; what became of it, in words, or nothing when the
  /// person left before asking.
  onDone: (said: string) => void;
}) {
  const form = useRef<HTMLFormElement>(null);
  const [asked, setAsked] = useState<Asked | null>(null);
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const fields = useMemo(() => {
    try {
      return fieldsOf(asked ? asked.requested_schema : tool.input_schema);
    } catch (error) {
      // A shape this form cannot carry is said, never drawn as a box that
      // takes the wrong thing.
      return (error as Error).message;
    }
  }, [asked, tool]);
  const filled = (): Record<string, unknown> => {
    if (!Array.isArray(fields)) throw new Error(fields);
    if (form.current && !form.current.reportValidity()) throw new Error("Something in the form is not filled in as it asks.");
    return valueOf(fields, readRaw(form.current));
  };
  const act = async (action: "accept" | "decline" | "cancel") => {
    setProblem("");
    if (!asked && action !== "accept") {
      onDone("");
      return;
    }
    setBusy(true);
    try {
      const at = await connection();
      if (!at) throw new Error("The agent could not be reached.");
      if (!asked) {
        setAsked(
          await fetchJson<Asked>(`/api/connections/${part(at)}/apps/interactions`, post({ server_name: server, tool: tool.name, arguments: filled() })),
        );
        return;
      }
      await fetchJson(
        `/api/connections/${part(at)}/apps/interactions/${part(asked.interaction_id)}`,
        post(action === "accept" ? { action, content: filled() } : { action }),
      );
      onDone(action === "accept" ? `${toolNamed(tool)}: done.` : action === "decline" ? `${toolNamed(tool)}: you said no.` : `${toolNamed(tool)}: you left it.`);
    } catch (error) {
      setProblem((error as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <form
      className="k-card w-ask"
      aria-label={toolNamed(tool)}
      ref={form}
      onSubmit={(event) => {
        event.preventDefault();
        void act("accept");
      }}
    >
      <div className="k-stack w-close">
        <span className="k-title">{toolNamed(tool)}</span>
        <span className="k-caption">{asked ? `${server} asks` : `You ask ${server}; your agent takes no part in it`}</span>
        {(asked ? asked.message : tool.description) ? <span>{asked ? asked.message : tool.description}</span> : null}
      </div>
      {typeof fields === "string" ? <span className="k-caption k-is-danger">{fields}</span> : null}
      {/* What is asked next is another form: its fields begin empty. */}
      <div className="k-stack" key={asked?.interaction_id ?? "first"}>
        {Array.isArray(fields)
          ? fields.map((field) => (
              <label className="k-stack w-close elicitation-field" key={field.name}>
                <span className="k-caption">
                  {field.label}
                  {field.required ? "" : " (if you like)"}
                </span>
                <FieldInput field={field} />
                {field.description ? <span className="k-caption">{field.description}</span> : null}
              </label>
            ))
          : null}
      </div>
      <div className="k-inline w-tight">
        <button type="submit" className="k-btn k-primary" disabled={busy || typeof fields === "string"}>
          {asked ? "Answer" : "Go on"}
        </button>
        {asked ? (
          <button type="button" className="k-btn" disabled={busy} onClick={() => void act("decline")}>
            Not this
          </button>
        ) : null}
        <button type="button" className="k-btn k-quiet" disabled={busy} onClick={() => void act("cancel")}>
          Leave it
        </button>
      </div>
      {problem ? <span className="k-caption k-is-danger">{problem}</span> : null}
    </form>
  );
}
