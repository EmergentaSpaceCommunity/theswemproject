// A rung offers a person an action only when the rail can draw its form.
//
// The release action shipped with a nullable `destination` (`type:
// ["string","null"]`, what a Rust `Option<String>` serialises to), the flat
// form refused it, and the rail quietly replaced the button with the line
// "an agent does this". The rung's actions looked right in the envelope and
// the cycle test that read them passed; nobody could press anything. These
// pin the rule from both sides.
import assert from "node:assert/strict";
import { test } from "node:test";

import { fieldsOf } from "../src/agent/flatForm.ts";
import { formSchemaFor, undecidedStructuredArgument } from "../src/project/slots.ts";

/// The release tool's input schema as the kernel publishes it: the head
/// revision (an object the rung decides) and where the files go.
const releaseSchema = {
  type: "object",
  properties: {
    revision: {
      type: "object",
      properties: { schema: { type: "string" }, digest: { type: "string" } },
      required: ["schema", "digest"],
    },
    destination: {
      type: "string",
      description: "An existing empty workspace-relative directory that receives the files.",
    },
  },
  required: ["revision"],
};

const preset = { revision: { schema: "swem:model-revision@0.1", digest: "a".repeat(64) } };

test("the rail draws the release action once the rung has decided the revision", () => {
  const fields = fieldsOf(formSchemaFor(releaseSchema, preset));
  assert.deepEqual(
    fields.map((field) => [field.name, field.kind, field.required]),
    [["destination", "text", false]],
    "the person answers one question: where the files go",
  );
});

test("a property the rung has not decided must be a shape the form can draw", () => {
  // What the tool emitted before: `Option<String>` as a nullable type. The
  // form refuses it, so the rail offers no button at all - the failure the
  // envelope cannot show.
  const nullable = {
    ...releaseSchema,
    properties: { ...releaseSchema.properties, destination: { type: ["string", "null"] } },
  };
  assert.throws(
    () => fieldsOf(formSchemaFor(nullable, preset)),
    /destination/,
    "a nullable property must be refused loudly, not turn into a missing button",
  );
});

test("an object the rung has not decided is refused too", () => {
  // The flat form itself can draw a revision by its two properties, and for
  // a tool a person opens themselves that is right. A rung is not that: its
  // step names the revision it acts on, so the rail refuses the action
  // rather than asking a person for a schema URI and a digest.
  assert.equal(undecidedStructuredArgument(formSchemaFor(releaseSchema, {})), "the rung has not decided revision");
  assert.equal(undecidedStructuredArgument(formSchemaFor(releaseSchema, preset)), null);
});
