// What a project is made of, joined with what its server publishes - pure,
// so a test can ask what the tab strip and a rung's form would be.

import type { AppView, Envelope, SlotView, ToolView } from "./types";

export interface SlotTab {
  slot: string;
  module: string;
  /// What its kind of work is called, for the tab's own title.
  kind: string;
  /// The workbench that opens on this slot, or `null` when this build has
  /// no module of that name: the tab is shown and says so, never dropped.
  uri: string | null;
  description: string | null;
}

/// One tab per slot, in the envelope's order. A server without slots (an
/// older one, or a project not yet composed) yields no tabs: composing the
/// project is the next thing to do, not opening every workbench at once.
export function slotTabs(envelope: Envelope | null, apps: AppView[]): SlotTab[] {
  const slots: SlotView[] = envelope?.slots?.slots ?? [];
  return slots.map((slot) => {
    const app = slot.app_uri ? (apps.find((candidate) => candidate.uri === slot.app_uri) ?? null) : null;
    return {
      slot: slot.slot,
      module: slot.module,
      kind: slot.title || slot.module,
      uri: slot.app_uri,
      description: app?.description ?? null,
    };
  });
}

interface ObjectSchema {
  type?: string;
  properties?: Record<string, unknown>;
  required?: string[];
  [key: string]: unknown;
}

/// The tool's input schema minus the arguments the rung already decided:
/// what the person is asked, and only that.
export function formSchemaFor(inputSchema: unknown, preset: Record<string, unknown> | undefined): ObjectSchema {
  const schema = (inputSchema ?? {}) as ObjectSchema;
  const decided = new Set(Object.keys(preset ?? {}));
  const properties = Object.fromEntries(
    Object.entries(schema.properties ?? {}).filter(([name]) => !decided.has(name)),
  );
  const required = (schema.required ?? []).filter((name) => !decided.has(name));
  return { ...schema, properties, required };
}

/// A structured argument the rung left undecided, named; `null` when it
/// decided them all.
///
/// The flat form can draw an object by its properties, and for a tool a
/// person opens themselves that is the right answer. A rung is not that: its
/// step names the revision it acts on, and a rung that failed to would turn
/// into two boxes asking a person for a schema URI and a 64-character
/// digest. The rail says it cannot draw the action instead of drawing that.
export function undecidedStructuredArgument(schema: ObjectSchema): string | null {
  for (const [name, definition] of Object.entries(schema.properties ?? {})) {
    if ((definition as { type?: string }).type === "object") return `the rung has not decided ${name}`;
  }
  return null;
}

/// The tool a rung's action names, if the server declares it and lets a
/// person call it.
export function toolFor(tools: ToolView[], name: string): ToolView | null {
  const tool = tools.find((candidate) => candidate.name === name) ?? null;
  return tool && tool.visibility.includes("app") ? tool : null;
}
