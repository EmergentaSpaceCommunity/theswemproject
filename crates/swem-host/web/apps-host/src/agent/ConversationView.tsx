// The conversation, as the reduction in `events.ts` describes it.

import { useEffect, useRef } from "react";

import type { ArtifactDescriptor, ArtifactIssue, Message, ToolCard } from "./events.ts";
import { useSession } from "./store.ts";

const STATUS_WORDS: Record<ToolCard["status"], string> = {
  pending: "waiting",
  in_progress: "running",
  completed: "done",
  failed: "failed",
};

/// A tool the agent used: its title and how far it got on one line, what it
/// said or touched behind a fold, so a long turn stays readable.
function ToolCardView({ tool }: { tool: ToolCard }) {
  const detail = [...tool.content, ...tool.locations.map((path) => `→ ${path}`)];
  return (
    <article className="tool-card k-card" data-tool-call-id={tool.toolCallId} data-status={tool.status} data-kind={tool.kind ?? undefined}>
      <div className="tool-card-head">
        <span className="tool-card-title">{tool.title}</span>
        <span className="k-chip tool-card-status">{STATUS_WORDS[tool.status]}</span>
      </div>
      {detail.length > 0 ? (
        <details className="tool-card-detail">
          <summary className="k-caption">what it did</summary>
          <pre>{detail.join("\n")}</pre>
        </details>
      ) : null}
    </article>
  );
}

function MessageView({ message }: { message: Message }) {
  const label = message.role === "user" ? "You" : message.role === "thought" ? "Reasoning" : "Agent";
  const className = `message ${message.role}${message.quiet && message.role !== "thought" ? " thought" : ""}`;
  return (
    <article className={className}>
      <div className="message-label">{label}</div>
      <div className="message-body">{message.text}</div>
    </article>
  );
}

function ArtifactCard({ artifact }: { artifact: ArtifactDescriptor }) {
  const source = `/api/content/${encodeURIComponent(artifact.descriptor_id)}`;
  return (
    <article className="artifact-card k-card" data-descriptor-id={artifact.descriptor_id} data-artifact-name={artifact.name}>
      <strong>{artifact.name}</strong>
      <small style={{ display: "block" }}>
        {artifact.media_type} · {artifact.byte_length} bytes · {artifact.content_digest}
      </small>
      {artifact.media_type.startsWith("image/") ? <img src={source} alt={artifact.name} /> : null}
      {artifact.media_type.startsWith("audio/") ? <audio src={source} controls /> : null}
      {artifact.media_type.startsWith("video/") ? <video src={source} controls /> : null}
      <div className="artifact-actions">
        <a href={source} target="_blank" rel="noopener">
          Open
        </a>
        <a href={`${source}?download=1`}>Download</a>
      </div>
    </article>
  );
}

function UnavailableCard({ issue }: { issue: ArtifactIssue }) {
  return (
    <article className="artifact-card k-card k-danger unavailable" data-artifact-name={issue.name} data-reason={issue.reason}>
      <strong>Unavailable · {issue.name}</strong>
      <small style={{ display: "block" }}>
        {issue.media_type ? `${issue.media_type} · ` : ""}
        {issue.reason}
      </small>
    </article>
  );
}

export function ConversationView({ hidden }: { hidden: boolean }) {
  const { conversation } = useSession();
  const container = useRef<HTMLElement>(null);
  const streamed = conversation.items.map((item) => (item.kind === "message" ? item.message.text.length : 1)).join(",");
  // Follow the newest item, as a chat does; a person who scrolled up is
  // brought back only when something new arrives.
  useEffect(() => {
    container.current?.lastElementChild?.scrollIntoView({ block: "end" });
  }, [streamed]);
  return (
    <section id="conversation" aria-live="polite" hidden={hidden} ref={container}>
      {conversation.items.map((item, index) => {
        switch (item.kind) {
          case "message":
            return <MessageView message={item.message} key={index} />;
          case "artifact":
            return <ArtifactCard artifact={item.artifact} key={index} />;
          case "unavailable":
            return <UnavailableCard issue={item.issue} key={index} />;
          case "tool":
            return <ToolCardView tool={item.tool} key={index} />;
        }
      })}
    </section>
  );
}
