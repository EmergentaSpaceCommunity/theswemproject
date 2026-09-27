// What the host says about chats, as it says it.

export type ParticipantKind = "person" | "agent" | "guest" | "schedule";

export interface Participant {
  participant_id: string;
  kind: ParticipantKind;
  handle: string;
  name: string;
  /// A role of the palette: info, success, warning, danger.
  colour?: string;
  profile_id?: string;
  made_by?: string;
  retired: boolean;
}

export interface Chat {
  chat_id: string;
  title: string;
  created_by: string;
  answer_rule: string;
  reply_limit: number;
  agent_replies: number;
  members: Participant[];
  last_sequence?: number;
  last_at_ms?: number;
}

export interface ContentBlock {
  type: string;
  text?: string;
  name?: string;
  uri?: string;
  media_type?: string;
  content_ref?: string;
  byte_length?: number;
}

export interface Message {
  message_id: string;
  sequence: number;
  chat_id: string;
  sender_id: string;
  channel: string;
  channel_ref?: string;
  content: ContentBlock[];
  text: string;
  named: string[];
  created_ms?: number;
}

export type DeliveryState = "queued" | "running" | "done" | "stopped" | "failed" | "interrupted";

export interface Delivery {
  delivery_id: string;
  chat_id: string;
  agent_id: string;
  message_id: string;
  state: DeliveryState;
  outcome?: string | null;
}

export interface PermissionOption {
  optionId: string;
  name: string;
  kind: string;
}

export interface Question {
  question_id: string;
  chat_id: string;
  agent_id: string;
  delivery_id?: string | null;
  /// `permission`, `form` or `link`.
  kind: string;
  asked: { title?: string | null; tool_kind?: string | null; options?: PermissionOption[] };
  state: "waiting" | "answered" | "lapsed";
  answer?: { option?: string; name?: string | null; action?: string } | null;
}

/// One thing that happened, as the stream and a page of a chat carry it.
export interface Happened {
  sequence: number;
  chat_id?: string;
  /// The agent in whose session it happened, for what an engine said.
  agent_id?: string;
  kind: string;
  source: string;
  payload: unknown;
  at_ms?: number;
  /// What was said, when this is the place of a message.
  message?: Message;
}

export interface ChatPage {
  chat: Chat;
  events: Happened[];
  messages: Message[];
  more: boolean;
  deliveries: Delivery[];
  questions: Question[];
}

export interface Now {
  head: number;
  owner: Participant;
  participants: Participant[];
  chats: Chat[];
  deliveries: Delivery[];
  questions: Question[];
}
