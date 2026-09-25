// The conversation as a reduction over the route ledger.
//
// The shell holds no conversation of its own: every message, artifact card,
// turn outcome and refusal count is a function of the events the host has
// delivered. This file is that function, with no DOM in it, so it can be
// driven by a test with a list of events and asked what a person would see.

export interface SurfaceEvent {
  sequence: number;
  kind: string;
  source: string;
  payload: unknown;
}

export interface Message {
  role: "user" | "assistant" | "thought";
  text: string;
  /// `thought` messages and tool-use notes are de-emphasised.
  quiet: boolean;
}

export interface ArtifactDescriptor {
  descriptor_id: string;
  name: string;
  media_type: string;
  byte_length: number;
  content_digest: string;
}

export interface ArtifactIssue {
  name: string;
  media_type?: string;
  reason: string;
}

/// One tool the agent used, as a card: what it was called, how far it got,
/// and what it said or touched, folded under the title.
export interface ToolCard {
  toolCallId: string;
  title: string;
  kind: string | null;
  status: "pending" | "in_progress" | "completed" | "failed";
  content: string[];
  locations: string[];
}

/// One line of the agent's plan for the turn, as ACP carries it.
export interface PlanEntry {
  content: string;
  priority: string | null;
  status: "pending" | "in_progress" | "completed";
}

/// A slash command the agent offers, from `available_commands_update`.
export interface AgentCommand {
  name: string;
  description: string;
}

/// What the agent has used of its window, when it says (an extension some
/// agents send; absent for the rest).
export interface Usage {
  used: number | null;
  size: number | null;
  cost: number | null;
  currency: string | null;
}

export type ConversationItem =
  | { kind: "message"; message: Message }
  | { kind: "artifact"; artifact: ArtifactDescriptor }
  | { kind: "unavailable"; issue: ArtifactIssue }
  | { kind: "tool"; tool: ToolCard };

/// What the session's end looked like, in words a person can read; the
/// payload itself stays in the diagnostics list where it belongs.
export interface Terminal {
  status: "finished" | "failed";
  /// A sentence.
  summary: string;
  /// The exact host message, for the diagnostics rail and for a retry flow.
  detail: string | null;
  /// True when the failure is the agent asking to be authenticated first.
  needsAuthentication: boolean;
}

export interface Conversation {
  items: ConversationItem[];
  /// Index into `items` of the assistant message still being streamed.
  streaming: number | null;
  turnOutcome: string;
  terminal: Terminal | null;
  refusals: number;
  /// The permission dialog closes on a turn's end.
  turnEnded: boolean;
  /// The agent's plan for the current turn; replaced whole on every update.
  plan: PlanEntry[];
  /// The mode the session is in, by the agent's id for it, when it says.
  mode: string | null;
  /// The slash commands the agent offers.
  commands: AgentCommand[];
  usage: Usage | null;
  /// Bumped when the agent's config options changed on the wire, so the page
  /// reads them again from the host rather than guessing from the event.
  optionsVersion: number;
}

export const emptyConversation = (): Conversation => ({
  items: [],
  streaming: null,
  turnOutcome: "",
  terminal: null,
  refusals: 0,
  turnEnded: false,
  plan: [],
  mode: null,
  commands: [],
  usage: null,
  optionsVersion: 0,
});

interface ContentBlock {
  type?: string;
  text?: string;
  name?: string;
  uri?: string;
  resource?: { uri?: string };
}

/// One ACP content block as a line of text.
export function contentText(content: unknown): string {
  const block = (content ?? {}) as ContentBlock;
  switch (block.type) {
    case "text":
      return block.text ?? "";
    case "resource_link":
      return `↗ ${block.name ?? block.uri ?? ""}`;
    case "image":
      return "Image attachment";
    case "audio":
      return "Audio attachment";
    case "resource":
      return `Attachment · ${(block.resource?.uri ?? "").split("/").pop() || "unnamed"}`;
    default:
      return "";
  }
}

const AUTH_MARKER = "requires authentication";

/// The host's terminal payload (`{status, error?}`), read for a person.
export function describeTerminal(payload: unknown): Terminal {
  const value = (payload ?? {}) as { status?: string; error?: string; termination?: string };
  if (value.status === "failed") {
    const detail = value.error ?? "";
    const needsAuthentication = detail.includes(AUTH_MARKER);
    return {
      status: "failed",
      summary: needsAuthentication
        ? "The agent needs to be authenticated before it can start a session."
        : "The session ended with an error.",
      detail: detail || null,
      needsAuthentication,
    };
  }
  return {
    status: "finished",
    summary: value.termination ? `Session ended (${value.termination}).` : "Session ended.",
    detail: null,
    needsAuthentication: false,
  };
}

function push(conversation: Conversation, item: ConversationItem): Conversation {
  return { ...conversation, items: [...conversation.items, item] };
}

/// Apply one event. Pure: the previous conversation is not mutated.
export function reduceEvent(conversation: Conversation, event: SurfaceEvent): Conversation {
  const payload = (event.payload ?? {}) as Record<string, unknown>;
  switch (event.kind) {
    case "host/prompt_submitted": {
      const blocks = Array.isArray(payload["content"]) ? (payload["content"] as unknown[]) : [];
      const text = blocks.map(contentText).filter(Boolean).join("\n");
      return {
        ...push(conversation, { kind: "message", message: { role: "user", text, quiet: false } }),
        streaming: null,
        turnEnded: false,
      };
    }
    case "host/artifact_available":
      return push(conversation, { kind: "artifact", artifact: payload as unknown as ArtifactDescriptor });
    case "host/artifact_unavailable":
      return push(conversation, { kind: "unavailable", issue: payload as unknown as ArtifactIssue });
    case "acp/prompt_response":
      return {
        ...conversation,
        streaming: null,
        turnOutcome: `turn: ${String(payload["stop_reason"])} / ${String(payload["control_outcome"])}`,
        turnEnded: true,
      };
    case "host/session_terminal":
      return { ...conversation, terminal: describeTerminal(event.payload) };
    case "host/app_tool_call":
      return payload["decision"] === "refused"
        ? { ...conversation, refusals: conversation.refusals + 1 }
        : conversation;
    case "acp/session_new": {
      // The session's opening answer: the modes the agent has, and whether it
      // offers anything to configure - the options themselves are read from
      // the host, which holds them as they stand.
      const modes = payload["modes"] as { currentModeId?: string } | null | undefined;
      return {
        ...conversation,
        mode: modes?.currentModeId ?? conversation.mode,
        optionsVersion: conversation.optionsVersion + 1,
      };
    }
    case "acp/session_update": {
      const update = (payload["update"] ?? null) as
        | {
            sessionUpdate?: string;
            content?: ContentBlock | { type?: string; content?: ContentBlock }[];
            title?: string;
            toolCallId?: string;
            kind?: string;
            status?: string;
            locations?: { path?: string }[];
            entries?: { content?: string; priority?: string; status?: string }[];
            currentModeId?: string;
            availableCommands?: { name?: string; description?: string }[];
            used?: number;
            size?: number;
            cost?: number;
            currency?: string;
          }
        | null;
      if (!update || typeof update !== "object") return conversation;
      if (update.sessionUpdate === "tool_call" || update.sessionUpdate === "tool_call_update") {
        return withToolUpdate(conversation, update);
      }
      if (update.sessionUpdate === "plan") {
        const entries = Array.isArray(update.entries) ? update.entries : [];
        return {
          ...conversation,
          plan: entries.map((entry) => ({
            content: entry.content ?? "",
            priority: entry.priority ?? null,
            status: planStatus(entry.status),
          })),
        };
      }
      if (update.sessionUpdate === "current_mode_update") {
        return { ...conversation, mode: update.currentModeId ?? conversation.mode };
      }
      if (update.sessionUpdate === "available_commands_update") {
        const commands = Array.isArray(update.availableCommands) ? update.availableCommands : [];
        return {
          ...conversation,
          commands: commands
            .filter((command) => typeof command.name === "string" && command.name)
            .map((command) => ({ name: command.name ?? "", description: command.description ?? "" })),
        };
      }
      if (update.sessionUpdate === "config_option_update") {
        return { ...conversation, optionsVersion: conversation.optionsVersion + 1 };
      }
      if (update.sessionUpdate === "usage_update") {
        return {
          ...conversation,
          usage: {
            used: typeof update.used === "number" ? update.used : null,
            size: typeof update.size === "number" ? update.size : null,
            cost: typeof update.cost === "number" ? update.cost : null,
            currency: typeof update.currency === "string" ? update.currency : null,
          },
        };
      }
      if (!isBlock(update.content)) return conversation;
      if (update.sessionUpdate === "agent_message_chunk" && update.content?.type === "text") {
        const chunk = update.content.text ?? "";
        if (conversation.streaming !== null) {
          const items = conversation.items.slice();
          const current = items[conversation.streaming];
          if (current?.kind === "message") {
            items[conversation.streaming] = {
              kind: "message",
              message: { ...current.message, text: current.message.text + chunk },
            };
            return { ...conversation, items };
          }
        }
        const items = [...conversation.items, { kind: "message" as const, message: { role: "assistant" as const, text: chunk, quiet: false } }];
        return { ...conversation, items, streaming: items.length - 1 };
      }
      if (update.sessionUpdate === "agent_thought_chunk" && update.content?.type === "text") {
        return push(conversation, { kind: "message", message: { role: "thought", text: update.content.text ?? "", quiet: true } });
      }
      return conversation;
    }
    default:
      return conversation;
  }
}

function isBlock(value: unknown): value is ContentBlock | undefined {
  return value === undefined || (typeof value === "object" && value !== null && !Array.isArray(value));
}

function planStatus(status: unknown): PlanEntry["status"] {
  return status === "in_progress" || status === "completed" ? status : "pending";
}

function toolStatus(status: unknown, fallback: ToolCard["status"]): ToolCard["status"] {
  return status === "pending" || status === "in_progress" || status === "completed" || status === "failed"
    ? status
    : fallback;
}

/// A tool call, or an update to one, as a card: the first word about a call
/// makes the card, every later one finds it by id and adds to it. An update
/// for a call nobody announced still makes a card, because the agent did use
/// it and a person should see that.
function withToolUpdate(
  conversation: Conversation,
  update: {
    sessionUpdate?: string;
    content?: unknown;
    title?: string;
    toolCallId?: string;
    kind?: string;
    status?: string;
    locations?: { path?: string }[];
  },
): Conversation {
  const id = update.toolCallId ?? `tool-${conversation.items.length}`;
  const lines = Array.isArray(update.content)
    ? (update.content as { type?: string; content?: unknown }[])
        .map((piece) => (piece?.type === "content" ? contentText(piece.content) : ""))
        .filter(Boolean)
    : [];
  const locations = Array.isArray(update.locations)
    ? update.locations.map((location) => location?.path ?? "").filter(Boolean)
    : [];
  const index = conversation.items.findIndex((item) => item.kind === "tool" && item.tool.toolCallId === id);
  if (index === -1) {
    return push(conversation, {
      kind: "tool",
      tool: {
        toolCallId: id,
        title: update.title ?? id,
        kind: update.kind ?? null,
        status: toolStatus(update.status, "pending"),
        content: lines,
        locations,
      },
    });
  }
  const items = conversation.items.slice();
  const current = items[index];
  if (current?.kind !== "tool") return conversation;
  items[index] = {
    kind: "tool",
    tool: {
      ...current.tool,
      title: update.title ?? current.tool.title,
      kind: update.kind ?? current.tool.kind,
      status: toolStatus(update.status, current.tool.status),
      content: [...current.tool.content, ...lines],
      locations: [...current.tool.locations, ...locations.filter((path) => !current.tool.locations.includes(path))],
    },
  };
  return { ...conversation, items };
}

/// Every http(s) `uri` reachable inside a payload, for the diagnostics list.
export function linksIn(value: unknown, into: { href: string; label: string }[] = []): { href: string; label: string }[] {
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    if (typeof record["uri"] === "string") {
      const label = typeof record["name"] === "string" ? record["name"] : record["uri"];
      into.push({ href: record["uri"], label });
    }
    for (const child of Object.values(record)) linksIn(child, into);
  }
  return into;
}
