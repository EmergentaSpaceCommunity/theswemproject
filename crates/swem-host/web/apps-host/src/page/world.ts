// Everything the page knows, kept where every part of it can read it.
//
// One stream from the host feeds it (`/api/stream`): the state whole when
// the page opens, then everything that happens, in the host's own order.
// Who there is, the chats, what they owe and what waits for an answer are
// kept here; what was said in a chat is kept per chat, and only for chats
// somebody opened.

import { at } from "../base.ts";
import { createStore, type StoreApi } from "zustand/vanilla";

import { fetchJson } from "../http.ts";
import { emptyTimeline, takeIn, type Timeline } from "./timeline.ts";
import type { Chat, ChatPage, Delivery, Happened, Now, Participant, Question } from "./types.ts";

export interface World {
  /// Whether the host has said how things stand.
  ready: boolean;
  /// Whether the stream is down; the page says so and keeps what it has.
  lost: boolean;
  owner: Participant | null;
  participants: Record<string, Participant>;
  chats: Record<string, Chat>;
  /// What is owed and not ended, by delivery.
  deliveries: Record<string, Delivery>;
  /// What waits for an answer, by question.
  questions: Record<string, Question>;
  /// Per agent, why it last could not answer; nothing once it has.
  failed: Record<string, string>;
  /// The place the record had reached when the page began to follow it:
  /// what is after it happened while the page was open.
  since: number;
}

export interface ChatState {
  /// `loading` until the chat was read; `failed` says why it was not.
  status: "loading" | "ready" | "failed";
  problem: string;
  timeline: Timeline;
  /// Whether there is more before what was read, and where what was read begins.
  more: boolean;
  begins: number;
  /// What happened while the chat was being read, taken in after it.
  waiting: Happened[];
}

export const world = createStore<World>(() => ({
  ready: false,
  lost: false,
  owner: null,
  participants: {},
  chats: {},
  deliveries: {},
  questions: {},
  failed: {},
  since: 0,
}));

const opened = new Map<string, StoreApi<ChatState>>();
const PAGE = 400;

const byId = <T,>(rows: T[], id: (row: T) => string): Record<string, T> =>
  Object.fromEntries(rows.map((row) => [id(row), row]));

function stands(now: Now, anew = false): void {
  world.setState({
    ...(anew || !world.getState().ready ? { since: now.head } : {}),
    ready: true,
    lost: false,
    owner: now.owner,
    participants: byId(now.participants, (one) => one.participant_id),
    chats: byId(now.chats, (chat) => chat.chat_id),
    deliveries: byId(now.deliveries, (delivery) => delivery.delivery_id),
    questions: byId(now.questions, (question) => question.question_id),
  });
}

/// Read a chat's members and name again; a chat nobody here knew yet is
/// learnt this way.
async function learn(chatId: string): Promise<void> {
  try {
    const page = await fetchJson<ChatPage>(`/api/chats/${encodeURIComponent(chatId)}?limit=1`);
    world.setState((state) => ({
      chats: { ...state.chats, [chatId]: page.chat },
      participants: { ...state.participants, ...byId(page.chat.members, (one) => one.participant_id) },
    }));
  } catch {
    // The next thing that happens in it asks again.
  }
}

function happened(event: Happened): void {
  const chatId = event.chat_id;
  if (!chatId) return;
  const state = world.getState();
  const chat = state.chats[chatId];
  const payload = (event.payload ?? {}) as Record<string, unknown>;
  const patch: Partial<World> = {};
  if (chat) {
    patch.chats = {
      ...state.chats,
      [chatId]: {
        ...chat,
        last_sequence: event.sequence,
        last_at_ms: event.at_ms ?? chat.last_at_ms,
        ...(event.kind === "chat/renamed" && typeof payload["title"] === "string" && payload["title"] ? { title: payload["title"] } : {}),
      },
    };
  }
  if (event.kind === "chat/delivery") {
    const delivery = { ...(payload as unknown as Delivery), chat_id: chatId };
    const { [delivery.delivery_id]: _was, ...rest } = state.deliveries;
    patch.deliveries = delivery.state === "queued" || delivery.state === "running" ? { ...rest, [delivery.delivery_id]: delivery } : rest;
    if (delivery.state === "failed") patch.failed = { ...state.failed, [delivery.agent_id]: delivery.outcome ?? "" };
    if (delivery.state === "done") {
      const { [delivery.agent_id]: _mended, ...others } = state.failed;
      patch.failed = others;
    }
  }
  if (event.kind === "chat/question") {
    const question = { ...(payload as unknown as Question), chat_id: chatId };
    const { [question.question_id]: _was, ...rest } = state.questions;
    patch.questions = question.state === "waiting" ? { ...rest, [question.question_id]: question } : rest;
  }
  world.setState(patch);
  // A chat that is new here, that somebody joined, that was renamed to
  // nothing, or that just got its first words is read again for its name.
  const named = Boolean(chat?.title);
  // And one whose members or rules changed, or where agents may be
  // answering each other: how far their chain has gone is the chat's.
  const several = (chat?.members.filter((member) => member.kind === "agent").length ?? 0) > 1;
  const changed = ["chat/joined", "chat/left", "chat/retired", "chat/ruled", "chat/started", "chat/held"].includes(event.kind);
  if (!chat || changed || (event.kind === "chat/renamed" && !payload["title"]) || (!named && event.message) || (several && event.message)) {
    void learn(chatId);
  }
  const store = opened.get(chatId);
  if (!store) return;
  const current = store.getState();
  if (current.status === "loading") store.setState({ waiting: [...current.waiting, event] });
  else store.setState({ timeline: takeIn(current.timeline, event) });
}

let stream: EventSource | null = null;

/// Follow the host. Called once, when the page opens; a stream that went
/// down comes back by itself, from the place it had reached.
export function follow(): void {
  if (stream) return;
  stream = new EventSource(at("/api/stream"));
  const whole = (message: MessageEvent<string>) => stands(JSON.parse(message.data) as Now);
  stream.addEventListener("state", whole);
  stream.addEventListener("reset", (message) => {
    // The record is another one: nothing read from the old one holds.
    opened.clear();
    stands(JSON.parse((message as MessageEvent<string>).data) as Now, true);
  });
  stream.addEventListener("event", (message) => happened(JSON.parse((message as MessageEvent<string>).data) as Happened));
  stream.addEventListener("open", () => world.setState({ lost: false }));
  stream.addEventListener("error", () => world.setState({ lost: true }));
}

function joined(page: ChatPage): Happened[] {
  const said = new Map(page.messages.map((message) => [message.sequence, message]));
  return page.events.map((event) => ({ ...event, message: said.get(event.sequence) }));
}

/// What was said in a chat, read when somebody opens it and kept while the
/// page is open.
export function chatStore(chatId: string): StoreApi<ChatState> {
  const known = opened.get(chatId);
  if (known) return known;
  const store = createStore<ChatState>(() => ({
    status: "loading",
    problem: "",
    timeline: emptyTimeline(),
    more: false,
    begins: 0,
    waiting: [],
  }));
  opened.set(chatId, store);
  void fetchJson<ChatPage>(`/api/chats/${encodeURIComponent(chatId)}?limit=${PAGE}`)
    .then((page) => {
      const read = joined(page);
      const timeline = [...read, ...store.getState().waiting].reduce(takeIn, emptyTimeline());
      store.setState({ status: "ready", timeline, more: page.more, begins: read[0]?.sequence ?? 0, waiting: [] });
    })
    .catch((error: Error) => store.setState({ status: "failed", problem: error.message }));
  return store;
}

/// Read what was said before what the page has of a chat.
export async function readEarlier(chatId: string): Promise<void> {
  const store = opened.get(chatId);
  if (!store) return;
  const { begins, more, timeline } = store.getState();
  if (!more || !begins) return;
  const page = await fetchJson<ChatPage>(`/api/chats/${encodeURIComponent(chatId)}?limit=${PAGE}&before=${begins}`);
  const earlier = joined(page).reduce(takeIn, emptyTimeline());
  store.setState({
    timeline: { ...timeline, items: [...earlier.items, ...timeline.items] },
    more: page.more,
    begins: page.events[0]?.sequence ?? begins,
  });
}

const json = (body: unknown): RequestInit => ({
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify(body),
});

let said = 0;
/// A name for a message that no other message of this page has, so saying
/// it again after a lost answer is not saying it twice.
const once = (): string => `page-${Date.now().toString(36)}-${(said += 1)}-${Math.random().toString(36).slice(2, 8)}`;

export interface Saying {
  text: string;
  content_refs?: string[];
  blocks?: unknown[];
  /// What an App said the person is looking at.
  context?: { server_name: string; content: unknown[] };
}

export const act = {
  /// Ask the host who there is again: somebody was made.
  async people(): Promise<void> {
    const people = await fetchJson<{ owner: Participant; participants: Participant[] }>("/api/people");
    world.setState({ owner: people.owner, participants: byId(people.participants, (one) => one.participant_id) });
  },
  async startChat(agents: string[], title = ""): Promise<Chat> {
    const chat = await fetchJson<Chat>("/api/chats", json({ agents, title }));
    world.setState((state) => ({ chats: { ...state.chats, [chat.chat_id]: chat } }));
    return chat;
  },
  /// Say something in a chat. It answers how many agents it is for.
  async say(chatId: string, saying: Saying): Promise<number> {
    const said = await fetchJson<{ deliveries?: unknown[] }>(`/api/chats/${encodeURIComponent(chatId)}/messages`, json({ ...saying, client_ref: once() }));
    return said.deliveries?.length ?? 0;
  },
  /// Bring an agent into a chat, by its profile.
  async bring(chatId: string, profile: string): Promise<void> {
    const chat = await fetchJson<Chat>(`/api/chats/${encodeURIComponent(chatId)}/members`, json({ agent: profile }));
    world.setState((state) => ({ chats: { ...state.chats, [chat.chat_id]: chat } }));
  },
  /// Let a guest into a chat: somebody who wrote to a bot of yours.
  async letIn(chatId: string, guest: string): Promise<void> {
    const chat = await fetchJson<Chat>(`/api/chats/${encodeURIComponent(chatId)}/members`, json({ guest }));
    world.setState((state) => ({ chats: { ...state.chats, [chat.chat_id]: chat } }));
  },
  async takeOut(chatId: string, participant: string): Promise<void> {
    const chat = await fetchJson<Chat>(`/api/chats/${encodeURIComponent(chatId)}/members/${encodeURIComponent(participant)}`, { method: "DELETE" });
    world.setState((state) => ({ chats: { ...state.chats, [chat.chat_id]: chat } }));
  },
  /// Set a chat's rules, or call it something else.
  async change(chatId: string, change: { title?: string; answer_rule?: string; reply_limit?: number }): Promise<void> {
    const chat = await fetchJson<Chat>(`/api/chats/${encodeURIComponent(chatId)}`, { ...json(change), method: "PATCH" });
    world.setState((state) => ({ chats: { ...state.chats, [chat.chat_id]: chat } }));
  },
  async stop(chatId: string, agentId?: string): Promise<void> {
    await fetchJson(`/api/chats/${encodeURIComponent(chatId)}/stop`, json(agentId ? { agent_id: agentId } : {}));
  },
  async answer(questionId: string, answer: unknown): Promise<void> {
    await fetchJson(`/api/questions/${encodeURIComponent(questionId)}/answer`, json(answer));
  },
  async sleep(agentId: string): Promise<void> {
    await fetchJson(`/api/people/${encodeURIComponent(agentId)}/sleep`, json({}));
  },
  async rename(chatId: string, title: string): Promise<void> {
    await fetchJson(`/api/chats/${encodeURIComponent(chatId)}`, { ...json({ title }), method: "PATCH" });
  },
};

/// The chats of one agent with the person, and the chats of several, the
/// one that moved last first.
export function chatsOf(state: World, agentId: string | null): Chat[] {
  return Object.values(state.chats)
    .filter((chat) => {
      const agents = chat.members.filter((member) => member.kind === "agent");
      return agentId === null ? agents.length > 1 : agents.some((agent) => agent.participant_id === agentId);
    })
    .sort((left, right) => (right.last_sequence ?? 0) - (left.last_sequence ?? 0) || right.chat_id.localeCompare(left.chat_id));
}

export type Doing = "working" | "asking" | "waiting" | "ready";

/// What an agent is doing, in a chat or anywhere.
export function doing(state: World, agentId: string, chatId?: string): Doing {
  const here = <T extends { agent_id: string; chat_id: string }>(row: T) =>
    row.agent_id === agentId && (chatId === undefined || row.chat_id === chatId);
  if (Object.values(state.questions).some(here)) return "asking";
  const owed = Object.values(state.deliveries).filter(here);
  if (owed.some((delivery) => delivery.state === "running")) return "working";
  return owed.length > 0 ? "waiting" : "ready";
}
