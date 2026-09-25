// The flat form is the only shape a Level 1 host may ask a person to fill.
// These tests pin what it renders and what it refuses.
import assert from "node:assert/strict";
import { test } from "node:test";

import { fieldsOf, initialRaw, valueOf } from "../src/agent/flatForm.ts";

const schema = {
  type: "object",
  required: ["strategy", "iterations"],
  properties: {
    strategy: { type: "string", oneOf: [{ const: "fast", title: "Fast" }, { const: "careful", title: "Careful", description: "slower" }] },
    iterations: { type: "integer", minimum: 1, maximum: 9, default: 3 },
    gain_db: { type: "number" },
    normalize: { type: "boolean" },
    stems: { type: "array", items: { enum: ["drums", "bass", "vox"] }, minItems: 1, maxItems: 2 },
    email: { type: "string", format: "email" },
  },
};

test("every property becomes a typed field in declaration order", () => {
  const fields = fieldsOf(schema);
  assert.deepEqual(fields.map((field) => [field.name, field.kind, field.required]), [
    ["strategy", "select", true],
    ["iterations", "number", true],
    ["gain_db", "number", false],
    ["normalize", "boolean", false],
    ["stems", "multiselect", false],
    ["email", "text", false],
  ]);
  assert.equal(fields[0].choices[1].description, "slower");
  assert.equal(fields[5].inputType, "email");
  assert.deepEqual(initialRaw(fields).iterations, "3");
});

test("answers coerce to what the schema types them as", () => {
  const fields = fieldsOf(schema);
  const value = valueOf(fields, { strategy: "careful", iterations: "4", gain_db: "-3.5", normalize: "true", stems: ["vox"], email: "" });
  assert.deepEqual(value, { strategy: "careful", iterations: 4, gain_db: -3.5, normalize: true, stems: ["vox"] });
});

test("a missing required answer and an out-of-bounds selection are refused by name", () => {
  const fields = fieldsOf(schema);
  assert.throws(() => valueOf(fields, { strategy: "", iterations: "2" }), /strategy is required/);
  assert.throws(() => valueOf(fields, { strategy: "fast", iterations: "2", stems: ["drums", "bass", "vox"] }), /stems selection count/);
});

test("a shape the flat form cannot carry is refused, not rendered as a text box", () => {
  assert.throws(() => fieldsOf({ properties: { free: { type: "array", items: { type: "string" } } } }), /unsupported ACP array property free/);
});

/// The launch form of `amend_composition_interactively`, as the Cycle has
/// published it since wire 1.17: the parent named by schema and digest, the
/// same way the operation it presents names it.
const structuredSchema = {
  type: "object",
  properties: {
    revision: {
      type: "object",
      properties: { schema: { type: "string" }, digest: { type: "string" } },
      required: ["schema", "digest"],
    },
    introduced_by: { type: "string" },
  },
  required: ["revision", "introduced_by"],
};

test("a structured argument is drawn as its own properties, and answered into place", () => {
  // Before this, the dialog opened with no inputs at all and said the shape
  // was unavailable: nobody could launch the form from the workbench.
  const fields = fieldsOf(structuredSchema);
  assert.deepEqual(
    fields.map((field) => [field.name, field.label, field.required]),
    [
      ["revision.schema", "revision schema", true],
      ["revision.digest", "revision digest", true],
      ["introduced_by", "introduced_by", true],
    ],
  );
  assert.deepEqual(
    valueOf(fields, {
      "revision.schema": "swem:model-revision@0.1",
      "revision.digest": "a".repeat(64),
      introduced_by: "swem.actor:workbench-operator",
    }),
    {
      revision: { schema: "swem:model-revision@0.1", digest: "a".repeat(64) },
      introduced_by: "swem.actor:workbench-operator",
    },
  );
});

test("an object with nothing declared inside it is still refused", () => {
  assert.throws(() => fieldsOf({ properties: { free: { type: "object" } } }), /unsupported MCP form property free/);
  assert.throws(
    () => fieldsOf({ properties: { outer: { type: "object", properties: { bad: { type: ["string", "null"] } } } } }),
    /unsupported MCP form property outer\.bad/,
  );
});
