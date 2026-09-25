// The inputs of a flat form, shared by every place a person answers one:
// an agent's elicitation, a rung's action on the project.
//
// The inputs are uncontrolled and read when the form is answered, not
// mirrored keystroke by keystroke: what the form holds is what is sent,
// whichever way it got there.

import type { Field } from "./flatForm.ts";

export type Raw = Record<string, string | string[]>;

export function readRaw(form: HTMLFormElement | null): Raw {
  const raw: Raw = {};
  if (!form) return raw;
  for (const input of form.querySelectorAll<HTMLInputElement | HTMLSelectElement>("[data-name]")) {
    const name = input.dataset["name"] ?? "";
    raw[name] =
      input instanceof HTMLSelectElement && input.multiple ? Array.from(input.selectedOptions).map((option) => option.value) : input.value;
  }
  return raw;
}

export function FieldInput({ field }: { field: Field }) {
  const common = { "data-name": field.name, "data-type": field.schemaType, required: field.required };
  const initial = field.default;
  if (field.kind === "select" || field.kind === "boolean") {
    const choices = field.kind === "boolean" ? [{ value: "true", label: "true" }, { value: "false", label: "false" }] : (field.choices ?? []);
    return (
      <select {...common} defaultValue={initial === undefined ? "" : String(initial)}>
        {field.required ? null : <option value="">—</option>}
        {choices.map((choice) => (
          <option value={choice.value} title={choice.description} key={choice.value}>
            {choice.label}
          </option>
        ))}
      </select>
    );
  }
  if (field.kind === "multiselect") {
    return (
      <select {...common} multiple data-min-items={field.minItems} data-max-items={field.maxItems} defaultValue={Array.isArray(initial) ? initial.map(String) : []}>
        {(field.choices ?? []).map((choice) => (
          <option value={choice.value} title={choice.description} key={choice.value}>
            {choice.label}
          </option>
        ))}
      </select>
    );
  }
  // A free text a person writes at length - an intent, a reasoning - is a
  // textarea; a bounded or formatted one stays a single line.
  const multiline =
    field.kind === "text" &&
    (field.inputType ?? "text") === "text" &&
    field.maxLength === undefined &&
    field.pattern === undefined &&
    /(^|_)(text|reasoning|description|body|intent)$/.test(field.name);
  if (multiline) {
    return <textarea {...common} rows={4} defaultValue={initial === undefined ? "" : String(initial)} />;
  }
  return (
    <input
      {...common}
      type={field.kind === "number" ? "number" : (field.inputType ?? "text")}
      step={field.kind === "number" ? (field.integer ? "1" : "any") : undefined}
      min={field.min}
      max={field.max}
      minLength={field.minLength}
      maxLength={field.maxLength}
      pattern={field.pattern}
      defaultValue={initial === undefined ? "" : String(initial)}
    />
  );
}

