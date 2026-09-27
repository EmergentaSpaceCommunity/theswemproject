// A chat: what was said in it, by whom, and the place to say something.
//
// The thread and the composer are the chat library's; what a message looks
// like, who it is from and what an agent did between its words are drawn
// here from the chat's own timeline.

import {
  AssistantRuntimeProvider,
  ComposerPrimitive,
  MessagePrimitive,
  ThreadPrimitive,
  useAui,
  useAuiState,
  useExternalStoreRuntime,
  type ThreadMessageLike,
} from "@assistant-ui/react";
import { useMemo, useRef, useState } from "react";
import { useStore } from "zustand";

import type { PlanEntry, ToolCard } from "../agent/events.ts";
import { Arrow, Check, Chevron, Clip, Clock, Cross, Laptop, Square, Wrench } from "./icons.tsx";
import { Prose } from "./Prose.tsx";
import { grouped, type Group, type Item } from "./timeline.ts";
import type { Chat, Message, Participant, Question } from "./types.ts";
import { Avatar, StateWord, when } from "./who.tsx";
import { act, chatStore, doing, readEarlier, world, type Saying } from "./world.ts";

const TOOL_WORDS: Record<ToolCard["status"], string> = {
  pending: "Waiting",
  in_progress: "Working",
  completed: "Done",
  failed: "Failed",
};
const TOOL_DOTS: Record<ToolCard["status"], string> = {
  pending: "",
  in_progress: "k-busy",
  completed: "k-ready",
  failed: "k-danger",
};

function Tool({ tool }: { tool: ToolCard }) {
  const detail = [...tool.locations, ...tool.content].filter(Boolean);
  return (
    <details className={`k-toolcall${tool.status === "failed" ? " k-danger" : ""}`}>
      <summary>
        <span className="k-muted"><Wrench /></span>
        <span className="k-two">
          <span>{tool.title}</span>
          {tool.locations[0] ? <span className="k-mono k-muted">{tool.locations[0]}</span> : null}
        </span>
        <span className="k-status">
          <span className={`k-dot k-small ${TOOL_DOTS[tool.status]}`} />
          <span>{TOOL_WORDS[tool.status]}</span>
        </span>
        {detail.length > 0 ? <span className="k-muted"><Chevron size={14} /></span> : null}
      </summary>
      {detail.length > 0 ? (
        <div>
          <pre>{detail.join("\n")}</pre>
        </div>
      ) : null}
    </details>
  );
}

function Plan({ entries }: { entries: PlanEntry[] }) {
  return (
    <div className="k-card w-plan">
      <span className="k-eyebrow">Plan</span>
      {entries.map((entry, index) => (
        <div className={`k-inline w-tight${entry.status === "in_progress" ? " k-name" : ""}${entry.status === "pending" ? " k-muted" : ""}`} key={index}>
          {entry.status === "completed" ? (
            <span className="k-is-success"><Check /></span>
          ) : (
            <span className={`k-dot k-small w-plan-dot${entry.status === "in_progress" ? " k-busy" : ""}`} />
          )}
          <span>{entry.content}</span>
        </div>
      ))}
    </div>
  );
}

/// What an agent asks before it does something. It waits for whoever
/// answers, here or anywhere else the chat is open.
function Ask({ question, agent }: { question: Question; agent: Participant | undefined }) {
  const [problem, setProblem] = useState("");
  const [sent, setSent] = useState(false);
  const name = agent?.name ?? "The agent";
  if (question.state !== "waiting") {
    const said = question.state === "lapsed" ? "Nobody answered before the turn ended." : question.answer?.name || question.answer?.action || "Answered.";
    return (
      <div className="k-notice">
        <span>{question.asked.title ? `${question.asked.title}: ` : ""}{said}</span>
      </div>
    );
  }
  if (question.kind !== "permission") {
    return (
      <div className="k-card k-asking w-ask" role="group" aria-label={`${name} asks`}>
        <span className="k-title k-is-warning">{name} asks for something it needs from you</span>
        <span className="k-caption">{question.kind === "link" ? "It is a link to open." : "It is a short form."}</span>
      </div>
    );
  }
  const options = question.asked.options ?? [];
  const choose = (option: string) => {
    setSent(true);
    setProblem("");
    act.answer(question.question_id, { option }).catch((error: Error) => {
      setSent(false);
      setProblem(error.message);
    });
  };
  return (
    <div className="k-card k-asking w-ask" role="group" aria-label={`${name} asks`}>
      <div className="k-stack w-close">
        <span className="k-title k-is-warning">{name} asks before it goes on</span>
        {question.asked.title ? <code className="k-mono">{question.asked.title}</code> : null}
      </div>
      <div className="k-inline w-tight">
        {options.map((option, index) => (
          <button type="button" className={`k-btn${index === 0 && option.kind.startsWith("allow") ? " k-primary" : ""}`} disabled={sent} key={option.optionId} onClick={() => choose(option.optionId)}>
            {option.name}
          </button>
        ))}
      </div>
      {problem ? <span className="k-caption k-is-danger">{problem}</span> : null}
    </div>
  );
}

function Attached({ message }: { message: Message }) {
  const files = message.content.filter((block) => block.type !== "text");
  if (files.length === 0) return null;
  return (
    <div className="k-inline w-tight">
      {files.map((block, index) => (
        <span className="k-chip" key={index}>
          <Clip size={12} />
          <span>{block.name ?? block.uri?.split("/").pop() ?? "attachment"}</span>
        </span>
      ))}
    </div>
  );
}

const THROUGH: Record<string, { words: string; sign: typeof Clock }> = {
  schedule: { words: "on time", sign: Clock },
  editor: { words: "from your editor", sign: Laptop },
};

function One({ item, people }: { item: Item; people: Record<string, Participant> }) {
  switch (item.kind) {
    case "said":
      return (
        <>
          <Prose text={item.message.text} settled />
          <Attached message={item.message} />
        </>
      );
    case "text":
      return <Prose text={item.text} settled={item.settled} />;
    case "thought":
      return <div className="k-caption w-thought">{item.text}</div>;
    case "tool":
      return <Tool tool={item.tool} />;
    case "plan":
      return <Plan entries={item.entries} />;
    case "file":
      return (
        <a className="k-chip" href={`/api/content/${encodeURIComponent(item.file.descriptor_id)}`} target="_blank" rel="noopener noreferrer">
          <Clip size={12} />
          <span>{item.file.name}</span>
        </a>
      );
    case "ask":
      return <Ask question={item.question} agent={people[item.by]} />;
    case "note":
      return <div className={`k-notice${item.tone === "muted" ? "" : ` k-${item.tone}`}`}>{item.text}</div>;
  }
}

function Said() {
  const group = useAuiState((state) => (state.message.metadata?.custom as { group?: Group } | undefined)?.group);
  const people = useStore(world, (state) => state.participants);
  const owner = useStore(world, (state) => state.owner);
  if (!group) return null;
  const first = group.items[0];
  const by = group.by ? people[group.by] : undefined;
  const mine = first?.kind === "said" && group.by === owner?.participant_id;
  if (mine && first?.kind === "said") {
    const through = THROUGH[first.message.channel];
    return (
      <MessagePrimitive.Root className="w-mine">
        {through ? (
          <span className="k-status">
            <through.sign size={13} />
            <span>Said {through.words}</span>
          </span>
        ) : null}
        <div className="k-bubble">
          <Prose text={first.message.text} settled />
          <Attached message={first.message} />
        </div>
      </MessagePrimitive.Root>
    );
  }
  if (!group.by) {
    return (
      <MessagePrimitive.Root className="w-said-by-nobody">
        {group.items.map((item) => <One item={item} people={people} key={item.key} />)}
      </MessagePrimitive.Root>
    );
  }
  return (
    <MessagePrimitive.Root className="w-msg">
      <Avatar who={by} />
      <div className="w-col">
        <div className="w-from">
          <span className="k-name">{by?.name ?? "Somebody"}</span>
          <span className="k-caption">{when(group.at)}</span>
        </div>
        {group.items.map((item) => <One item={item} people={people} key={item.key} />)}
      </div>
    </MessagePrimitive.Root>
  );
}

async function handOver(file: File): Promise<string> {
  const response = await fetch(`/api/content?name=${encodeURIComponent(file.name)}`, {
    method: "POST",
    headers: { "content-type": file.type || "application/octet-stream" },
    body: file,
  });
  const value = (await response.json()) as { descriptor_id?: string; error?: string };
  if (!response.ok || !value.descriptor_id) throw new Error(value.error ?? String(response.status));
  return value.descriptor_id;
}

function Composer({
  to,
  running,
  onSay,
  onStop,
}: {
  to: string;
  running: boolean;
  onSay: (saying: Saying) => Promise<void>;
  onStop: () => void;
}) {
  const aui = useAui();
  const text = useAuiState((state) => state.composer.text);
  const [files, setFiles] = useState<File[]>([]);
  const [problem, setProblem] = useState("");
  const [sending, setSending] = useState(false);
  const picker = useRef<HTMLInputElement>(null);
  const nothing = text.trim() === "" && files.length === 0;
  const send = async () => {
    if (nothing || sending) return;
    setSending(true);
    setProblem("");
    try {
      const content_refs = await Promise.all(files.map(handOver));
      await onSay({ text, content_refs });
      aui.composer().setText("");
      setFiles([]);
    } catch (error) {
      setProblem((error as Error).message);
    } finally {
      setSending(false);
    }
  };
  return (
    <ComposerPrimitive.Root
      className="w-composer"
      onSubmit={(event) => {
        event.preventDefault();
        void send();
      }}
    >
      {files.length > 0 ? (
        <div className="k-inline w-tight">
          {files.map((file, index) => (
            <span className="k-chip" key={`${file.name}-${index}`}>
              <Clip size={12} />
              <span>{file.name}</span>
              <button type="button" className="w-chip-x" aria-label={`Take ${file.name} off`} onClick={() => setFiles(files.filter((_, at) => at !== index))}>
                <Cross size={12} />
              </button>
            </span>
          ))}
        </div>
      ) : null}
      <ComposerPrimitive.Input
        className="w-composer-input"
        aria-label={`Message ${to}`}
        placeholder={`Message ${to}`}
        rows={2}
        submitOnEnter={false}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
            event.preventDefault();
            void send();
          }
        }}
      />
      <div className="k-spread">
        <div className="k-inline w-tight">
          <input
            ref={picker}
            type="file"
            multiple
            hidden
            onChange={(event) => {
              setFiles([...files, ...Array.from(event.target.files ?? [])]);
              event.target.value = "";
            }}
          />
          <button type="button" className="k-btn k-quiet" onClick={() => picker.current?.click()}>
            <Clip />
            <span>Attach</span>
          </button>
          {problem ? <span className="k-caption k-is-danger">{problem}</span> : null}
        </div>
        <div className="k-inline w-tight">
          {running ? (
            <button type="button" className="k-btn" onClick={onStop}>
              <Square />
              <span>Stop</span>
            </button>
          ) : null}
          <button type="submit" className="k-btn k-primary" disabled={nothing || sending}>
            <Arrow />
            <span>Send</span>
          </button>
        </div>
      </div>
    </ComposerPrimitive.Root>
  );
}

function Thread({
  groups,
  to,
  running,
  more,
  onEarlier,
  onSay,
  onStop,
  children,
}: {
  groups: Group[];
  to: string;
  running: boolean;
  more: boolean;
  onEarlier: () => void;
  onSay: (saying: Saying) => Promise<void>;
  onStop: () => void;
  children?: React.ReactNode;
}) {
  const owner = useStore(world, (state) => state.owner);
  const messages = useMemo<ThreadMessageLike[]>(
    () =>
      groups.map((group) => {
        const first = group.items[0];
        const mine = first?.kind === "said" && group.by === owner?.participant_id;
        const text = group.items.map((item) => ("text" in item ? item.text : item.kind === "said" ? item.message.text : "")).join("\n");
        const open = group.items.some((item) => item.kind === "text" && !item.settled);
        return {
          id: group.key,
          role: mine ? "user" : "assistant",
          content: [{ type: "text", text }],
          ...(mine ? {} : { status: open ? { type: "running" as const } : { type: "complete" as const, reason: "stop" as const } }),
          metadata: { custom: { group } },
        };
      }),
    [groups, owner],
  );
  const runtime = useExternalStoreRuntime({
    messages,
    convertMessage: (message: ThreadMessageLike) => message,
    isRunning: false,
    onNew: async () => {},
  });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <ThreadPrimitive.Root className="w-thread">
        <ThreadPrimitive.Viewport className="w-stream">
          {more ? (
            <button type="button" className="k-btn k-quiet w-earlier" onClick={onEarlier}>
              Show what was said before
            </button>
          ) : null}
          {children}
          <ThreadPrimitive.Messages components={{ Message: Said }} />
        </ThreadPrimitive.Viewport>
        <Composer to={to} running={running} onSay={onSay} onStop={onStop} />
      </ThreadPrimitive.Root>
    </AssistantRuntimeProvider>
  );
}

export function ChatView({ chat }: { chat: Chat }) {
  const store = chatStore(chat.chat_id);
  const state = useStore(store);
  const owner = useStore(world, (one) => one.owner);
  const busy = useStore(world, (one) => Object.values(one.deliveries).some((delivery) => delivery.chat_id === chat.chat_id));
  const groups = useMemo(() => grouped(state.timeline.items), [state.timeline.items]);
  const others = chat.members.filter((member) => member.participant_id !== owner?.participant_id && member.kind !== "schedule");
  const to = others.length === 1 ? (others[0]?.name ?? "") : (chat.title || "everyone");
  const several = chat.members.filter((member) => member.kind === "agent").length > 1;
  return (
    <Thread
      groups={groups}
      to={several ? `${to}. Name who it is for with @` : to}
      running={busy}
      more={state.more}
      onEarlier={() => void readEarlier(chat.chat_id)}
      onSay={(saying) => act.say(chat.chat_id, saying)}
      onStop={() => void act.stop(chat.chat_id)}
    >
      {state.status === "loading" ? <div className="k-caption">Reading the chat…</div> : null}
      {state.status === "failed" ? <div className="k-notice k-danger">The chat could not be read: {state.problem}</div> : null}
      {state.status === "ready" && groups.length === 0 ? (
        <div className="k-empty">
          <span className="k-title">Nothing has been said here yet.</span>
          <span>Write to {to} below.</span>
        </div>
      ) : null}
    </Thread>
  );
}

/// The place to begin a chat with an agent that has none: the first thing
/// said makes the chat.
export function FirstWords({ agent, onBegun }: { agent: Participant; onBegun: (chat: Chat) => void }) {
  const say = async (saying: Saying) => {
    if (!agent.profile_id) throw new Error(`${agent.name} has no agent behind it any more`);
    const chat = await act.startChat([agent.profile_id]);
    onBegun(chat);
    await act.say(chat.chat_id, saying);
  };
  return (
    <Thread groups={[]} to={agent.name} running={false} more={false} onEarlier={() => {}} onSay={say} onStop={() => {}}>
      <div className="k-empty">
        <span className="k-title">Start a chat with {agent.name}.</span>
        <span>Say what you want done. It works where it lives and asks before what you told it to ask about.</span>
      </div>
    </Thread>
  );
}

export { StateWord, doing };
