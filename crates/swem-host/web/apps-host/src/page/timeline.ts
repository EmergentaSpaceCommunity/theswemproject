// A chat as a person reads it, made from what happened in it.
//
// Everything on the page about a chat is a function of the events the host
// delivered, in the one order the host keeps. This file is that function,
// with no DOM in it, so a test can give it events and ask what a person
// would see.

import { contentText, type AgentCommand, type ArtifactDescriptor, type PlanEntry, type ToolCard, type Usage } from "../agent/events.ts";
import type { Delivery, Happened, Message, Question } from "./types.ts";

export type Tone = "info" | "success" | "warning" | "danger" | "muted";

export type Item =
  /// What somebody said, whole: a person, a schedule, a guest.
  | { kind: "said"; key: string; by: string; at?: number; message: Message }
  /// What an agent is saying or said, a stretch of it between what it did.
  | { kind: "text"; key: string; by: string; at?: number; text: string; settled: boolean }
  | { kind: "thought"; key: string; by: string; text: string }
  | { kind: "tool"; key: string; by: string; tool: ToolCard }
  | { kind: "plan"; key: string; by: string; entries: PlanEntry[] }
  | { kind: "file"; key: string; by: string; file: ArtifactDescriptor }
  | { kind: "ask"; key: string; by: string; question: Question }
  | { kind: "note"; key: string; by?: string; tone: Tone; text: string };

/// An App a tool of an agent brought: where in the record, and which call
/// of the session it was.
export interface Brought {
  place: number;
  call: number;
}

export interface Timeline {
  items: Item[];
  /// The last place taken in.
  through: number;
  /// Per agent, where its turn's open stretch of text and its plan are.
  open: Record<string, { text?: string; plan?: string }>;
  /// Per agent, what it offers to be asked with a slash.
  commands: Record<string, AgentCommand[]>;
  /// Per agent, how much of its memory it has used, when it says.
  usage: Record<string, Usage>;
  /// Per agent, the place at which what its session lets a person choose
  /// last changed; what it offers is read from the session again then.
  offers: Record<string, number>;
  /// Per agent, the App a tool of its last brought.
  brought: Record<string, Brought>;
}

export const emptyTimeline = (): Timeline => ({ items: [], through: 0, open: {}, commands: {}, usage: {}, offers: {}, brought: {} });

function replaced(items: Item[], key: string, make: (old: Item) => Item): Item[] {
  const at = items.findIndex((item) => item.key === key);
  if (at === -1) return items;
  const next = items.slice();
  next[at] = make(items[at] as Item);
  return next;
}

function closed(timeline: Timeline, agent: string): Timeline {
  const open = timeline.open[agent];
  if (!open) return timeline;
  const items = open.text
    ? replaced(timeline.items, open.text, (item) => (item.kind === "text" ? { ...item, settled: true } : item))
    : timeline.items;
  const { [agent]: _gone, ...rest } = timeline.open;
  return { ...timeline, items, open: rest };
}

/// A stretch of text ends where the agent does something: what it says
/// after is a new stretch, below what it did.
function stretchEnds(timeline: Timeline, agent: string): Timeline {
  const open = timeline.open[agent];
  if (!open?.text) return timeline;
  return {
    ...timeline,
    items: replaced(timeline.items, open.text, (item) => (item.kind === "text" ? { ...item, settled: true } : item)),
    open: { ...timeline.open, [agent]: { ...open, text: undefined } },
  };
}

interface Update {
  sessionUpdate?: string;
  content?: unknown;
  title?: string;
  toolCallId?: string;
  kind?: string;
  status?: string;
  locations?: { path?: string }[];
  entries?: { content?: string; priority?: string; status?: string }[];
  availableCommands?: { name?: string; description?: string }[];
  used?: number;
  size?: number;
  cost?: number;
  currency?: string;
}

/// What a tool is called, as a person reads it. An engine names a tool of
/// a server by the way it addresses it (`mcp__notes__save_note`); said in
/// words that is what it does and whose it is ("save note · notes").
export function toolCalled(title: string | undefined): string | undefined {
  const named = /^mcp__(.+?)__(.+)$/.exec(title ?? "");
  if (!named) return title;
  const [, server, tool] = named;
  return `${(tool ?? "").replace(/_/g, " ")} · ${server}`;
}

const toolStatus = (status: unknown, fallback: ToolCard["status"]): ToolCard["status"] =>
  status === "pending" || status === "in_progress" || status === "completed" || status === "failed" ? status : fallback;

function withTool(timeline: Timeline, agent: string, sequence: number, update: Update): Timeline {
  const id = update.toolCallId ?? `at-${sequence}`;
  const key = `tool:${agent}:${id}`;
  const lines = Array.isArray(update.content)
    ? (update.content as { type?: string; content?: unknown }[])
        .map((piece) => (piece?.type === "content" ? contentText(piece.content) : ""))
        .filter(Boolean)
    : [];
  const locations = Array.isArray(update.locations)
    ? update.locations.map((location) => location?.path ?? "").filter(Boolean)
    : [];
  if (!timeline.items.some((item) => item.key === key)) {
    const ended = stretchEnds(timeline, agent);
    return {
      ...ended,
      items: [
        ...ended.items,
        {
          kind: "tool",
          key,
          by: agent,
          tool: {
            toolCallId: id,
            title: toolCalled(update.title) ?? id,
            kind: update.kind ?? null,
            status: toolStatus(update.status, "pending"),
            content: lines,
            locations,
          },
        },
      ],
    };
  }
  return {
    ...timeline,
    items: replaced(timeline.items, key, (item) =>
      item.kind !== "tool"
        ? item
        : {
            ...item,
            tool: {
              ...item.tool,
              title: toolCalled(update.title) ?? item.tool.title,
              kind: update.kind ?? item.tool.kind,
              status: toolStatus(update.status, item.tool.status),
              content: [...item.tool.content, ...lines],
              locations: [...item.tool.locations, ...locations.filter((path) => !item.tool.locations.includes(path))],
            },
          },
    ),
  };
}

function withUpdate(timeline: Timeline, agent: string, happened: Happened, update: Update): Timeline {
  switch (update.sessionUpdate) {
    case "agent_message_chunk": {
      const block = update.content as { type?: string; text?: string } | undefined;
      if (block?.type !== "text" || !block.text) return timeline;
      const open = timeline.open[agent];
      if (open?.text) {
        return {
          ...timeline,
          items: replaced(timeline.items, open.text, (item) =>
            item.kind === "text" ? { ...item, text: item.text + block.text } : item,
          ),
        };
      }
      const key = `text:${happened.sequence}`;
      return {
        ...timeline,
        items: [...timeline.items, { kind: "text", key, by: agent, at: happened.at_ms, text: block.text, settled: false }],
        open: { ...timeline.open, [agent]: { ...open, text: key } },
      };
    }
    case "agent_thought_chunk": {
      const block = update.content as { type?: string; text?: string } | undefined;
      if (block?.type !== "text" || !block.text) return timeline;
      const last = timeline.items[timeline.items.length - 1];
      if (last?.kind === "thought" && last.by === agent) {
        return { ...timeline, items: replaced(timeline.items, last.key, (item) => (item.kind === "thought" ? { ...item, text: item.text + block.text } : item)) };
      }
      const ended = stretchEnds(timeline, agent);
      return { ...ended, items: [...ended.items, { kind: "thought", key: `thought:${happened.sequence}`, by: agent, text: block.text }] };
    }
    case "tool_call":
    case "tool_call_update":
      return withTool(timeline, agent, happened.sequence, update);
    case "plan": {
      const entries: PlanEntry[] = (Array.isArray(update.entries) ? update.entries : []).map((entry) => ({
        content: entry.content ?? "",
        priority: entry.priority ?? null,
        status: entry.status === "in_progress" || entry.status === "completed" ? entry.status : "pending",
      }));
      const open = timeline.open[agent];
      if (open?.plan) {
        return { ...timeline, items: replaced(timeline.items, open.plan, (item) => (item.kind === "plan" ? { ...item, entries } : item)) };
      }
      const ended = stretchEnds(timeline, agent);
      const key = `plan:${happened.sequence}`;
      return {
        ...ended,
        items: [...ended.items, { kind: "plan", key, by: agent, entries }],
        open: { ...ended.open, [agent]: { ...ended.open[agent], plan: key } },
      };
    }
    case "available_commands_update":
      return {
        ...timeline,
        commands: {
          ...timeline.commands,
          [agent]: (Array.isArray(update.availableCommands) ? update.availableCommands : [])
            .filter((command) => typeof command.name === "string" && command.name)
            .map((command) => ({ name: command.name ?? "", description: command.description ?? "" })),
        },
      };
    case "config_option_update":
    case "current_mode_update":
      return { ...timeline, offers: { ...timeline.offers, [agent]: happened.sequence } };
    case "usage_update":
      return {
        ...timeline,
        usage: {
          ...timeline.usage,
          [agent]: {
            used: typeof update.used === "number" ? update.used : null,
            size: typeof update.size === "number" ? update.size : null,
            cost: typeof update.cost === "number" ? update.cost : null,
            currency: typeof update.currency === "string" ? update.currency : null,
          },
        },
      };
    default:
      return timeline;
  }
}

const inWords = (payload: Record<string, unknown>): string => {
  const said = payload["in_words"];
  if (Array.isArray(said)) return said.filter((one) => typeof one === "string").join("; ");
  return typeof said === "string" ? said : "";
};

function withDelivery(timeline: Timeline, delivery: Delivery): Timeline {
  const key = `delivery:${delivery.delivery_id}`;
  const without = timeline.items.filter((item) => item.key !== key);
  const note = (tone: Tone, text: string): Timeline => ({
    ...closed(timeline, delivery.agent_id),
    items: [...closed({ ...timeline, items: without }, delivery.agent_id).items, { kind: "note", key, by: delivery.agent_id, tone, text }],
  });
  switch (delivery.state) {
    case "failed":
      return note("danger", delivery.outcome || "It could not answer.");
    case "interrupted":
      return note("warning", "It was answering when the Workbench stopped. Say it again to go on.");
    case "stopped":
      return note("muted", "Stopped.");
    case "done":
      return closed({ ...timeline, items: without }, delivery.agent_id);
    default:
      return { ...timeline, items: without };
  }
}

/// Take in one thing that happened. Pure: what was given is not changed.
/// Told the same thing twice, it is taken in once.
export function takeIn(timeline: Timeline, happened: Happened): Timeline {
  if (happened.sequence <= timeline.through) return timeline;
  const next = taken({ ...timeline, through: happened.sequence }, happened);
  return next;
}

function taken(timeline: Timeline, happened: Happened): Timeline {
  // What an engine replayed when its session was loaded was said before.
  if (happened.source === "native_replay") return timeline;
  const payload = (happened.payload ?? {}) as Record<string, unknown>;
  const agent = happened.agent_id ?? "";
  const said = happened.message;
  // An agent's message is the text of its turn, which was drawn as it came.
  if (said && said.sender_id !== agent) {
    return {
      ...timeline,
      items: [...timeline.items, { kind: "said", key: `said:${said.message_id}`, by: said.sender_id, at: said.created_ms ?? happened.at_ms, message: said }],
    };
  }
  switch (happened.kind) {
    case "acp/session_update": {
      const update = payload["update"] as Update | undefined;
      return update && typeof update === "object" && agent ? withUpdate(timeline, agent, happened, update) : timeline;
    }
    case "acp/prompt_response":
      return agent ? closed(timeline, agent) : timeline;
    case "acp/session_new":
    case "acp/session_load":
    case "acp/session_resume":
      return agent ? { ...timeline, offers: { ...timeline.offers, [agent]: happened.sequence } } : timeline;
    case "host/app_tool_observed": {
      // The call is brought once, when it begins; how it ended is given to
      // the App that is already open.
      const call = payload["cursor"];
      return agent && payload["phase"] === "request" && typeof call === "number"
        ? { ...timeline, brought: { ...timeline.brought, [agent]: { place: happened.sequence, call } } }
        : timeline;
    }
    case "host/artifact_available":
      return {
        ...timeline,
        items: [...timeline.items, { kind: "file", key: `file:${happened.sequence}`, by: agent, file: payload as unknown as ArtifactDescriptor }],
      };
    case "host/setup_changed":
      return { ...timeline, items: [...timeline.items, { kind: "note", key: `note:${happened.sequence}`, by: agent, tone: "info", text: `Since this chat began: ${inWords(payload)}.` }] };
    case "host/attachment_unavailable":
      return {
        ...timeline,
        items: [...timeline.items, { kind: "note", key: `note:${happened.sequence}`, by: agent, tone: "warning", text: `Attached, but not set up on this computer: ${inWords(payload)}. It works without.` }],
      };
    case "chat/held":
      return { ...timeline, items: [...timeline.items, { kind: "note", key: `note:${happened.sequence}`, tone: "warning", text: `${inWords(payload)}.` }] };
    case "chat/delivery":
      return withDelivery(timeline, payload as unknown as Delivery);
    case "chat/question": {
      const question = { ...(payload as unknown as Question), chat_id: happened.chat_id ?? "" };
      const key = `ask:${question.question_id}`;
      if (timeline.items.some((item) => item.key === key)) {
        return { ...timeline, items: replaced(timeline.items, key, (item) => (item.kind === "ask" ? { ...item, question } : item)) };
      }
      const ended = stretchEnds(timeline, question.agent_id);
      return { ...ended, items: [...ended.items, { kind: "ask", key, by: question.agent_id, question }] };
    }
    default:
      return timeline;
  }
}

/// A stretch of the timeline one participant is the author of, as it is
/// drawn: one name, then everything they said and did in a row.
export interface Group {
  key: string;
  by: string | null;
  at?: number;
  items: Item[];
}

export function grouped(items: Item[]): Group[] {
  const groups: Group[] = [];
  for (const item of items) {
    const by = item.by ?? null;
    const last = groups[groups.length - 1];
    // What a person says is a message of its own each time; what an agent
    // says and does in a row is one answer.
    if (last && by !== null && last.by === by && item.kind !== "said" && last.items[0]?.kind !== "said") {
      last.items.push(item);
    } else {
      groups.push({ key: item.key, by, at: "at" in item ? item.at : undefined, items: [item] });
    }
  }
  return groups;
}
