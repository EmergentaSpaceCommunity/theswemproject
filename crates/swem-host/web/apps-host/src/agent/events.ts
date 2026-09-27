// What an engine says in its updates, as the page names it: a tool it used,
// a line of its plan, what it offers to be asked, how much it has used.
// How a chat is made of them is `page/timeline.ts`.

export interface ArtifactDescriptor {
  descriptor_id: string;
  name: string;
  media_type: string;
  byte_length: number;
  content_digest: string;
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
