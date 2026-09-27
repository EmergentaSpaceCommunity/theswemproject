// Where a person writes to the agent: text and files. The two resource
// shapes a stock client also knows are fields of the dev drawer, read here
// by id when a turn is sent.

import { useRef, useState } from "react";

import { sessionStore, useSession } from "./store.ts";

const RESOURCE_FIELDS = ["link-uri", "link-name", "link-mime", "embed-uri", "embed-text"];

export function Composer({ hidden }: { hidden: boolean }) {
  const { routeId, writingAs, conversation } = useSession();
  // A chat that is open to read can be written to: the agent starts on the
  // first message.
  const connected = routeId !== null;
  // The text and resource fields are the person's draft, read when sent:
  // they are not mirrored into state keystroke by keystroke.
  const [files, setFiles] = useState<File[]>([]);
  const fileInput = useRef<HTMLInputElement>(null);
  const text = useRef<HTMLTextAreaElement>(null);
  // The slash commands the agent offers, shown while the draft starts with
  // "/" and narrowed by what follows it. Picking one puts it in the draft;
  // the turn still goes when the person sends it.
  const [slash, setSlash] = useState<string | null>(null);
  const commands = slash === null ? [] : conversation.commands.filter((command) => command.name.startsWith(slash));
  const pick = (name: string) => {
    if (text.current) {
      text.current.value = `/${name} `;
      text.current.focus();
    }
    setSlash(null);
  };
  const field = (id: string) => (document.getElementById(id) as HTMLInputElement | null)?.value.trim() ?? "";
  const send = async () => {
    // Who is writing is read the same way, by id, when the turn goes: the
    // field sits in the dev drawer, and a value put there while the drawer
    // is hidden never blurs, so it is taken here rather than on blur alone.
    const author = field("writing-as");
    if (author !== writingAs) sessionStore.setWritingAs(author);
    try {
      await sessionStore.sendPrompt({
        text: text.current?.value ?? "",
        files,
        link: { uri: field("link-uri"), name: field("link-name"), mimeType: field("link-mime") },
        embed: { uri: field("embed-uri"), text: (document.getElementById("embed-text") as HTMLInputElement | null)?.value ?? "" },
      });
    } catch {
      // The outcome line already says the turn failed.
      return;
    }
    if (text.current) text.current.value = "";
    setFiles([]);
    if (fileInput.current) fileInput.current.value = "";
    // The resource fields are part of the message that was just sent, so they
    // go with it. Left filled, one resource link rode every later turn.
    for (const id of RESOURCE_FIELDS) {
      const field = document.getElementById(id) as HTMLInputElement | null;
      if (field) field.value = "";
    }
  };
  return (
    <div id="composer-wrap" className="composer-wrap" hidden={hidden}>
      <div className="composer">
        {commands.length > 0 ? (
          <ul id="commands" className="commands k-card" role="listbox" aria-label="Commands">
            {commands.slice(0, 8).map((command) => (
              <li key={command.name}>
                <button className="command-row" role="option" onMouseDown={(event) => event.preventDefault()} onClick={() => pick(command.name)}>
                  <code>/{command.name}</code> <span className="k-muted">{command.description}</span>
                </button>
              </li>
            ))}
          </ul>
        ) : null}
        <textarea
          id="prompt-text"
          placeholder="Message your agent…"
          aria-label="Message"
          ref={text}
          onInput={(event) => {
            const draft = event.currentTarget.value;
            setSlash(draft.startsWith("/") && !draft.includes(" ") ? draft.slice(1) : null);
          }}
          onBlur={() => setSlash(null)}
          onKeyDown={(event) => {
            if (event.key === "Escape") setSlash(null);
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              if (commands.length > 0 && slash !== null) {
                const first = commands[0];
                if (first) pick(first.name);
                return;
              }
              if (connected) void send();
            }
          }}
        />
        <div id="attachment-list">{files.map((file) => file.name).join(" · ")}</div>
        <div className="composer-row">
          <label className="attach-label" htmlFor="file-input">
            Attach
          </label>
          <input
            id="file-input"
            type="file"
            multiple
            ref={fileInput}
            onChange={(event) => setFiles(Array.from(event.target.files ?? []))}
          />
          <button id="send" className="primary" disabled={!connected} onClick={() => void send()}>
            Send
          </button>
        </div>
      </div>
    </div>
  );
}
