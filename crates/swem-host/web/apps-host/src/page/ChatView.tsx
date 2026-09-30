// A chat: what was said in it, by whom, and the place to say something.
//
// The thread and the composer are the chat library's; what a message looks
// like, who it is from and what an agent did between its words are drawn
// here from the chat's own timeline.

import { at } from "../base.ts";
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
import { Select } from "@base-ui/react/select";
import { useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "zustand";

import type { AgentCommand, PlanEntry, ToolCard, Usage } from "../agent/events.ts";
import { FieldInput, readRaw } from "../agent/FlatFormFields.tsx";
import { fieldsOf, valueOf } from "../agent/flatForm.ts";
import type { ConfigChoice } from "../agent/session.ts";
import { fetchJson } from "../http.ts";
import { ChatApps } from "./ChatApps.tsx";
import { useOffers } from "./session.ts";
import { sessionStore, useSession } from "../agent/store.ts";
import { Arrow, Check, Chevron, Clip, Clock, Cross, Down, Laptop, Square, Tiles, Wrench } from "./icons.tsx";
import { Handles, Prose } from "./Prose.tsx";
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

/// A form an agent asks to be filled in, or a link it asks to be opened.
/// What it asks is read from its session, where it waits; what is typed
/// goes to the engine and is written down nowhere.
function AskForm({ question, name }: { question: Question; name: string }) {
  const [asked, setAsked] = useState<{ mode?: string; message?: string; url?: string; requestedSchema?: unknown } | null>(null);
  const [problem, setProblem] = useState("");
  const [sent, setSent] = useState(false);
  const form = useRef<HTMLFormElement>(null);
  useEffect(() => {
    let left = false;
    fetchJson<typeof asked>(`/api/questions/${encodeURIComponent(question.question_id)}`)
      .then((read) => {
        if (!left) setAsked(read);
      })
      .catch((error: Error) => {
        if (!left) setProblem(error.message);
      });
    return () => {
      left = true;
    };
  }, [question.question_id]);
  const fields = useMemo(() => {
    try {
      return asked?.mode === "form" ? fieldsOf(asked.requestedSchema ?? {}) : [];
    } catch (error) {
      return (error as Error).message;
    }
  }, [asked]);
  const answer = (action: "accept" | "decline" | "cancel") => {
    let body: Record<string, unknown> = { action };
    // Saying yes to a link is what opens it: nothing is opened before.
    if (action === "accept" && asked?.mode === "url" && asked.url) window.open(asked.url, "_blank", "noopener,noreferrer");
    if (action === "accept" && Array.isArray(fields) && asked?.mode === "form") {
      try {
        body = { action, content: valueOf(fields, readRaw(form.current)) };
      } catch (error) {
        setProblem((error as Error).message);
        return;
      }
    }
    setSent(true);
    setProblem("");
    act.answer(question.question_id, body).catch((error: Error) => {
      setSent(false);
      setProblem(error.message);
    });
  };
  return (
    <form
      className="k-card k-asking w-ask"
      aria-label={`${name} asks`}
      ref={form}
      onSubmit={(event) => {
        event.preventDefault();
        answer("accept");
      }}
    >
      <div className="k-stack w-close">
        <span className="k-title k-is-warning">{name} asks for something it needs from you</span>
        {asked?.message ? <span>{asked.message}</span> : null}
        {asked?.mode === "url" && asked.url ? (
          <span className="k-caption">
            It opens <code className="k-mono">{asked.url}</code>, at {new URL(asked.url).host}.
          </span>
        ) : null}
      </div>
      {typeof fields === "string" ? <span className="k-caption k-is-danger">{fields}</span> : null}
      {Array.isArray(fields)
        ? fields.map((field) => (
            <label className="k-stack w-close elicitation-field" key={field.name}>
              <span className="k-caption">
                {field.label}
                {field.required ? "" : " (if you like)"}
              </span>
              <FieldInput field={field} />
              {field.description ? <span className="k-caption">{field.description}</span> : null}
            </label>
          ))
        : null}
      <div className="k-inline w-tight">
        <button type="submit" className="k-btn k-primary" disabled={sent || asked === null}>
          {asked?.mode === "url" ? "Open it" : "Answer"}
        </button>
        <button type="button" className="k-btn" disabled={sent} onClick={() => answer("decline")}>
          Not this
        </button>
      </div>
      {problem ? <span className="k-caption k-is-danger">{problem}</span> : null}
    </form>
  );
}

/// Whose words the person is reading. Two questions can carry the same
/// sentence and mean different things: an agent saying what it intends to do
/// is a report, and the host saying what it is about to run is a fact about
/// the next thing that happens. A person answering cannot tell them apart
/// from the sentence, so the question says which it is.
function whose(askedBy: string | null | undefined): string {
  if (askedBy === "host_callback") return "This exact command runs here if you allow it.";
  if (askedBy === "uncorrelated_agent_report") return "Your agent's own words. It has not reported the step this belongs to.";
  return "Your agent's own words, for a step it has reported.";
}

/// What an option does, in the person's words rather than the wire's. The
/// name on the button is the agent's own text and the kind is what it means,
/// so both are shown: an option called "Yes" that refuses from now on is
/// something a person has to be able to see.
function kindWords(kind: string): string {
  if (kind === "allow_once") return "allows once";
  if (kind === "allow_always") return "allows from now on";
  if (kind === "reject_once") return "refuses once";
  if (kind === "reject_always") return "refuses from now on";
  return kind;
}

/// What an agent asks before it does something. It waits for whoever
/// answers, here or anywhere else the chat is open.
function Ask({ question, agent }: { question: Question; agent: Participant | undefined }) {
  const [problem, setProblem] = useState("");
  const [sent, setSent] = useState(false);
  const name = agent?.name ?? "The agent";
  if (question.state !== "waiting") {
    const said = question.state === "lapsed" ? "Nobody answered before the turn ended." : `You answered: ${question.answer?.name || question.answer?.action || "yes"}.`;
    return <div className="k-notice">{said}</div>;
  }
  if (question.kind !== "permission") return <AskForm question={question} name={name} />;
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
        <span className="k-caption">{whose(question.asked.asked_by)}</span>
      </div>
      <div className="k-inline w-tight">
        {options.map((option, index) => (
          <button
            type="button"
            className={`k-btn${index === 0 && option.kind.startsWith("allow") ? " k-primary" : ""}`}
            title={kindWords(option.kind)}
            disabled={sent}
            key={option.optionId}
            onClick={() => choose(option.optionId)}
          >
            <span>{option.name}</span>
            <span className="w-kind">{kindWords(option.kind)}</span>
          </button>
        ))}
      </div>
      {problem ? <span className="k-caption k-is-danger">{problem}</span> : null}
    </div>
  );
}

function Attached({ message }: { message: Message }) {
  const beside = message.content.filter((block) => block.type !== "text");
  if (beside.length === 0) return null;
  return (
    <div className="k-inline w-tight">
      {beside.map((block, index) =>
        block.type === "context" ? (
          <span className="k-chip" key={index}>
            <Tiles size={12} />
            <span>With what you were looking at in {(block as { server_name?: string }).server_name}</span>
          </span>
        ) : (
          <span className="k-chip" key={index}>
            <Clip size={12} />
            <span>{block.name ?? block.uri?.split("/").pop() ?? "attachment"}</span>
          </span>
        ),
      )}
    </div>
  );
}

/// How much of its memory an agent has used, when it says.
function used(usage: Usage | undefined): string {
  if (!usage || usage.used === null || usage.size === null || usage.size <= 0) return "";
  return `${Math.round((usage.used / usage.size) * 100)}% of its memory used`;
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
        <a className="k-chip" href={at(`/api/content/${encodeURIComponent(item.file.descriptor_id)}`)} target="_blank" rel="noopener noreferrer">
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
  const response = await fetch(at(`/api/content?name=${encodeURIComponent(file.name)}`), {
    method: "POST",
    headers: { "content-type": file.type || "application/octet-stream" },
    body: file,
  });
  const value = (await response.json()) as { descriptor_id?: string; error?: string };
  if (!response.ok || !value.descriptor_id) throw new Error(value.error ?? String(response.status));
  return value.descriptor_id;
}

/// One thing the session lets a person choose: its model, its mode.
function Offer({ label, value, choices, onChoose }: { label: string; value: string; choices: ConfigChoice[]; onChoose: (value: string) => void }) {
  if (choices.length === 0) return null;
  const shown = choices.find((choice) => choice.value === value)?.name ?? value;
  return (
    <Select.Root value={value} onValueChange={(next) => (typeof next === "string" ? onChoose(next) : undefined)}>
      <Select.Trigger className="k-btn k-quiet" aria-label={label} title={label}>
        <Select.Value>{shown}</Select.Value>
        <Select.Icon>
          <Down size={13} />
        </Select.Icon>
      </Select.Trigger>
      <Select.Portal>
        <Select.Positioner sideOffset={6} className="w-over">
          <Select.Popup className="k-menu">
            {choices.map((choice) => (
              <Select.Item className="k-menu-item" value={choice.value} key={choice.value}>
                <Select.ItemText>{choice.name}</Select.ItemText>
              </Select.Item>
            ))}
          </Select.Popup>
        </Select.Positioner>
      </Select.Portal>
    </Select.Root>
  );
}

function Composer({
  to,
  running,
  commands,
  usage,
  offers,
  apps,
  names = [],
  onSay,
  onStop,
}: {
  to: string;
  running: boolean;
  commands: AgentCommand[];
  usage: string;
  offers?: ReturnType<typeof useOffers>;
  apps?: { shown: boolean; toggle: () => void };
  /// Who can be named in what is written, in a chat of several.
  names?: Participant[];
  /// Says it, and answers with what there is to tell the person about it.
  onSay: (saying: Saying) => Promise<string | void>;
  onStop: () => void;
}) {
  const aui = useAui();
  const text = useAuiState((state) => state.composer.text);
  const { offered } = useSession();
  const [files, setFiles] = useState<File[]>([]);
  const [problem, setProblem] = useState("");
  const [sending, setSending] = useState(false);
  const picker = useRef<HTMLInputElement>(null);
  const nothing = text.trim() === "" && files.length === 0;
  // What the session lets a person choose, in the engine's own words: an
  // engine that offers no choice shows none.
  const options = offers?.offers?.options ?? [];
  const model = options.find((option) => option.category === "model" && option.kind === "select") ?? null;
  const modeOption = options.find((option) => option.category === "mode" && option.kind === "select") ?? null;
  const modeChoices = modeOption?.choices ?? offers?.offers?.modes?.available ?? [];
  const currentMode = modeOption ? String(modeOption.currentValue) : (offers?.offers?.modes?.current ?? "");
  const refused = (error: Error) => setProblem(`It did not take that: ${error.message}`);
  const choose = (option: string, value: string) => offers?.choose(option, value).catch(refused);
  const chooseMode = (value: string) => (modeOption ? offers?.choose(modeOption.id, value).catch(refused) : offers?.mode(value).catch(refused));
  // What the agent offers to be asked with a slash, while a slash is all
  // that has been typed.
  const asked = /^\/(\S*)$/.exec(text)?.[1];
  const offeredCommands = asked === undefined ? [] : commands.filter((command) => command.name.startsWith(asked)).slice(0, 8);
  // Who can be named, while a name is being written after `@`.
  const naming = names.length > 0 ? /(^|\s)@([a-z0-9_-]*)$/i.exec(text) : null;
  const offeredNames = naming ? names.filter((one) => one.handle.startsWith((naming[2] ?? "").toLowerCase()) || one.name.toLowerCase().startsWith((naming[2] ?? "").toLowerCase())).slice(0, 8) : [];
  const name = (handle: string) => {
    aui.composer().setText(`${text.slice(0, text.length - (naming?.[2] ?? "").length)}${handle} `);
  };
  const [note, setNote] = useState("");
  const send = async () => {
    if (nothing || sending) return;
    setSending(true);
    setProblem("");
    setNote("");
    try {
      const content_refs = await Promise.all(files.map(handOver));
      const told = await onSay({ text, content_refs, ...(offered ? { context: offered } : {}) });
      setNote(told ?? "");
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
      {offeredNames.length > 0 ? (
        <div className="k-menu w-commands" role="listbox" aria-label="Who it is for">
          {offeredNames.map((one) => (
            <button type="button" role="option" aria-selected="false" className="k-menu-item" key={one.participant_id} onClick={() => name(one.handle)}>
              <Avatar who={one} size="small" />
              <span className="k-name">{one.name}</span>
              <span className="k-mono k-muted">@{one.handle}</span>
            </button>
          ))}
        </div>
      ) : null}
      {note ? <div className="k-notice">{note}</div> : null}
      {offeredCommands.length > 0 ? (
        <div className="k-menu w-commands" role="listbox" aria-label="What it can be asked">
          {offeredCommands.map((command) => (
            <button type="button" role="option" aria-selected="false" className="k-menu-item" key={command.name} onClick={() => aui.composer().setText(`/${command.name} `)}>
              <span className="k-mono">/{command.name}</span>
              <span className="k-caption w-one-line">{command.description}</span>
            </button>
          ))}
        </div>
      ) : null}
      {offered || files.length > 0 ? (
        <div className="k-inline w-tight">
          {offered ? (
            <span className="k-chip" title={offered.content.map((block) => ("text" in block ? block.text : block.name)).join("\n")}>
              <Tiles size={12} />
              <span>What you are looking at in {offered.server_name}</span>
              <button type="button" className="w-chip-x" aria-label="Let go of it" onClick={() => void sessionStore.clearContext()}>
                <Cross size={12} />
              </button>
            </span>
          ) : null}
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
            // While a name is being picked, Enter picks the first.
            const first = offeredNames[0];
            if (first) name(first.handle);
            else void send();
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
          {model ? <Offer label="Model" value={String(model.currentValue)} choices={model.choices} onChoose={(value) => void choose(model.id, value)} /> : null}
          {modeChoices.length > 1 ? <Offer label="How it works" value={currentMode} choices={modeChoices} onChoose={(value) => void chooseMode(value)} /> : null}
          {apps ? (
            <button type="button" className="k-btn k-quiet" aria-pressed={apps.shown} onClick={apps.toggle}>
              <Tiles />
              <span>Apps</span>
            </button>
          ) : null}
          {problem ? <span className="k-caption k-is-danger">{problem}</span> : null}
        </div>
        <div className="k-inline w-tight">
          {usage ? <span className="k-caption">{usage}</span> : null}
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
  commands = [],
  usage = "",
  offers,
  apps,
  names,
  more,
  onEarlier,
  onSay,
  onStop,
  after,
  children,
}: {
  groups: Group[];
  to: string;
  running: boolean;
  commands?: AgentCommand[];
  usage?: string;
  offers?: ReturnType<typeof useOffers>;
  apps?: { shown: boolean; toggle: () => void };
  names?: Participant[];
  more: boolean;
  onEarlier: () => void;
  onSay: (saying: Saying) => Promise<string | void>;
  onStop: () => void;
  /// What is said under everything that was said.
  after?: React.ReactNode;
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
          {after}
        </ThreadPrimitive.Viewport>
        <Composer to={to} running={running} commands={commands} usage={usage} offers={offers} apps={apps} names={names} onSay={onSay} onStop={onStop} />
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
  const agents = chat.members.filter((member) => member.kind === "agent");
  const several = agents.length > 1;
  // What one agent offers and has used is said in its own chat; in a chat
  // of several it would be one agent's among others.
  const only = several ? undefined : agents[0];
  const offers = useOffers(chat.chat_id, only?.participant_id, only ? (state.timeline.offers[only.participant_id] ?? 0) : 0);
  const [appsShown, setAppsShown] = useState(false);
  const since = useStore(world, (now) => now.since);
  const broughtLast = only ? state.timeline.brought[only.participant_id] : undefined;
  const broughtNow = broughtLast && broughtLast.place > since ? broughtLast : undefined;
  const handles = useMemo(() => chat.members.filter((member) => member.kind !== "schedule").map((member) => member.handle), [chat.members]);
  const say = async (saying: Saying): Promise<string | void> => {
    const forHowMany = await act.say(chat.chat_id, saying);
    // In a chat of several a message is for who it names.
    if (several && forHowMany === 0) return "That named nobody, so nobody answers it. Write @ and pick who it is for.";
    return undefined;
  };
  // How far a chain of agents answering each other has gone.
  const chain =
    several && chat.agent_replies > 0
      ? chat.agent_replies >= chat.reply_limit
        ? `Agents have answered each other ${chat.agent_replies} times in a row. The chain waits for you: write, and it goes on.`
        : `Agents have answered each other ${chat.agent_replies} ${chat.agent_replies === 1 ? "time" : "times"} in a row. After ${chat.reply_limit} the chain waits for a person.`
      : "";
  return (
    <Handles.Provider value={handles}>
      <Thread
        groups={groups}
        names={several ? agents.filter((one) => !one.retired) : []}
        commands={only ? (state.timeline.commands[only.participant_id] ?? []) : []}
        usage={only ? used(state.timeline.usage[only.participant_id]) : ""}
        offers={only ? offers : undefined}
        apps={only ? { shown: appsShown, toggle: () => setAppsShown(!appsShown) } : undefined}
        to={several ? `${to}. Name who it is for with @` : to}
        running={busy}
        more={state.more}
        onEarlier={() => void readEarlier(chat.chat_id)}
        onSay={say}
        onStop={() => void act.stop(chat.chat_id)}
        after={chain ? <div className="k-notice w-chain">{chain}</div> : null}
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
      {only ? (
        <ChatApps
          chat={chat.chat_id}
          agent={only.participant_id}
          name={only.name}
          brought={broughtNow}
          shown={appsShown}
          onShow={() => setAppsShown(true)}
          onHide={() => setAppsShown(false)}
        />
      ) : null}
    </Handles.Provider>
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
    return undefined;
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
