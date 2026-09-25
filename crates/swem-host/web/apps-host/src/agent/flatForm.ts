// A flat MCP/ACP form schema, as a model a component renders and a value a
// person's answers coerce to.
//
// The elicitation spec permits primitives, enums and enum arrays only; that
// is what makes a Level 1 host possible, and it is also what makes this
// file small enough to test completely. A tool's own input schema is not an
// elicitation and may name a structured argument - a revision by schema and
// digest - so a property that is an object of properties is drawn as those
// properties. Anything outside that is refused with the property's path,
// never rendered as a text box that silently accepts the wrong thing.

export type FieldKind = "select" | "multiselect" | "boolean" | "text" | "number";

export interface Choice {
  value: string;
  label: string;
  description?: string;
}

export interface Field {
  name: string;
  label: string;
  kind: FieldKind;
  required: boolean;
  description?: string;
  choices?: Choice[];
  /// Only for `text`: the HTML input type.
  inputType?: string;
  min?: number;
  max?: number;
  minLength?: number;
  maxLength?: number;
  pattern?: string;
  integer?: boolean;
  minItems?: number;
  maxItems?: number;
  default?: unknown;
  /// The schema's own `type`, for coercion.
  schemaType: string;
}

interface Definition {
  type?: string;
  title?: string;
  description?: string;
  enum?: unknown[];
  oneOf?: { const?: unknown; title?: string; description?: string }[];
  items?: { enum?: unknown[]; anyOf?: { const?: unknown; title?: string; description?: string }[] };
  format?: string;
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
  pattern?: string;
  minItems?: number;
  maxItems?: number;
  default?: unknown;
  properties?: Record<string, Definition>;
  required?: string[];
}

const STRING_INPUTS: Record<string, string> = { email: "email", uri: "url", date: "date", "date-time": "datetime-local" };

function choicesOf(items: unknown[] | undefined, titled: { const?: unknown; title?: string; description?: string }[] | undefined): Choice[] | null {
  if (titled) {
    return titled.map((item) => {
      const choice: Choice = { value: String(item.const), label: item.title ?? String(item.const) };
      if (item.description) choice.description = item.description;
      return choice;
    });
  }
  if (items) return items.map((item) => ({ value: String(item), label: String(item) }));
  return null;
}

/// The fields of an object schema, in declaration order.
///
/// A property that is itself an object of properties becomes its own
/// properties, each on its own line and named by its path - `revision.digest`
/// rather than one box asking a person for JSON. That is the only nesting
/// this carries; everything else it cannot draw is still refused with the
/// property's path.
///
/// Throws with the property's path for anything the flat form cannot carry.
export function fieldsOf(schema: unknown): Field[] {
  return fieldsIn(schema, "", "", true);
}

function fieldsIn(schema: unknown, prefix: string, labelPrefix: string, inherited: boolean): Field[] {
  const object = (schema ?? {}) as { properties?: Record<string, Definition>; required?: unknown };
  const required = new Set(Array.isArray(object.required) ? (object.required as string[]) : []);
  const properties = object.properties && typeof object.properties === "object" ? object.properties : {};
  return Object.entries(properties).flatMap(([name, definition]) => {
    const path = prefix ? `${prefix}.${name}` : name;
    const title = definition.title ?? name;
    const label = labelPrefix ? `${labelPrefix} ${title}` : title;
    const asked = inherited && required.has(name);
    if (definition.type === "object") {
      // An object with no properties of its own is free-form: there is
      // nothing to draw, and a text box would accept the wrong thing.
      if (!definition.properties || Object.keys(definition.properties).length === 0) {
        throw new Error(`unsupported MCP form property ${path} (object)`);
      }
      return fieldsIn(definition, path, label, asked);
    }
    const base = {
      name: path,
      label,
      required: asked,
      schemaType: definition.type ?? "string",
    };
    const field: Field = { ...base, kind: "text" };
    if (definition.description) field.description = definition.description;
    if (definition.default !== undefined) field.default = definition.default;
    const enumChoices = choicesOf(definition.enum, definition.oneOf);
    if (enumChoices) {
      field.kind = "select";
      field.choices = enumChoices;
    } else if (definition.type === "array") {
      const choices = choicesOf(definition.items?.enum, definition.items?.anyOf);
      if (!choices) throw new Error(`unsupported ACP array property ${path}`);
      field.kind = "multiselect";
      field.choices = choices;
      if (definition.minItems !== undefined) field.minItems = definition.minItems;
      if (definition.maxItems !== undefined) field.maxItems = definition.maxItems;
    } else if (definition.type === "boolean") {
      field.kind = "boolean";
    } else if (definition.type === "string") {
      field.kind = "text";
      field.inputType = STRING_INPUTS[definition.format ?? ""] ?? "text";
      if (definition.minLength !== undefined) field.minLength = definition.minLength;
      if (definition.maxLength !== undefined) field.maxLength = definition.maxLength;
      if (definition.pattern) field.pattern = definition.pattern;
    } else if (definition.type === "integer" || definition.type === "number") {
      field.kind = "number";
      field.integer = definition.type === "integer";
      if (definition.minimum !== undefined) field.min = definition.minimum;
      if (definition.maximum !== undefined) field.max = definition.maximum;
    } else {
      throw new Error(`unsupported MCP form property ${path} (${definition.type})`);
    }
    return field;
  });
}

/// The answers as the schema types them. `raw` holds what the inputs hold:
/// strings, or string arrays for a multiselect.
///
/// Throws with the field's name when a required answer is missing or a
/// selection count is outside the schema's bounds.
export function valueOf(fields: Field[], raw: Record<string, string | string[]>): Record<string, unknown> {
  const value: Record<string, unknown> = {};
  /// A field's name is its path, so an answer to `revision.digest` goes
  /// inside `revision` and not beside it.
  const place = (answer: unknown, path: string) => {
    const steps = path.split(".");
    const last = steps.pop() ?? path;
    let here = value;
    for (const step of steps) {
      if (typeof here[step] !== "object" || here[step] === null) here[step] = {};
      here = here[step] as Record<string, unknown>;
    }
    here[last] = answer;
  };
  for (const field of fields) {
    const answer = raw[field.name];
    if (field.kind === "multiselect") {
      const selected = Array.isArray(answer) ? answer : [];
      const min = field.minItems ?? 0;
      const max = field.maxItems ?? Number.MAX_SAFE_INTEGER;
      if (selected.length < min || selected.length > max) {
        throw new Error(`${field.name} selection count is outside schema bounds`);
      }
      if (selected.length > 0 || field.required) place(selected, field.name);
      continue;
    }
    const text = typeof answer === "string" ? answer : "";
    if (text === "") {
      if (field.required) throw new Error(`${field.name} is required`);
      continue;
    }
    if (field.schemaType === "integer") place(Number.parseInt(text, 10), field.name);
    else if (field.schemaType === "number") place(Number(text), field.name);
    else if (field.schemaType === "boolean") place(text === "true", field.name);
    else place(text, field.name);
  }
  return value;
}

/// What the inputs start with: the schema defaults, as the inputs hold them.
export function initialRaw(fields: Field[]): Record<string, string | string[]> {
  const raw: Record<string, string | string[]> = {};
  for (const field of fields) {
    if (field.default === undefined) {
      raw[field.name] = field.kind === "multiselect" ? [] : "";
    } else if (field.kind === "multiselect") {
      raw[field.name] = Array.isArray(field.default) ? field.default.map(String) : [];
    } else {
      raw[field.name] = String(field.default);
    }
  }
  return raw;
}
