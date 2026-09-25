// A terminal, in the product, in the agent's own environment.
//
// Agents are signed in at a prompt: a device code, a browser handshake, a
// `login` subcommand. Without a terminal here, "configure your agent" means
// "open another terminal, find the right directory, export the right
// variables" - which is the product refusing to do its job. This runs a real
// pty on the host with the profile's working directory, the profile's agent
// home as HOME and the profile's typed vault in the environment, so what is
// signed in here is signed in for the agent.
//
// Bytes go over the same long poll the rest of the shell uses: output is a
// byte sequence a reader continues from, input is a POST. Nothing here is
// recorded - a terminal is live, and a secret typed at a prompt must not
// become a record.

import { useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import xtermCss from "@xterm/xterm/css/xterm.css?raw";

import type { TerminalView } from "./session.ts";
import { sessionStore, useSession } from "./store.ts";

/// xterm's own stylesheet, inlined once: the shell is a single script, and a
/// second file would have to be served and kept in step with it.
function useXtermStyle() {
  useEffect(() => {
    const id = "xterm-style";
    if (document.getElementById(id)) return;
    const style = document.createElement("style");
    style.id = id;
    style.textContent = xtermCss;
    document.head.append(style);
  }, []);
}

const decode = (base64: string): Uint8Array => {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
};

const encode = (text: string): string => {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
};

/// One line about a terminal, for a person choosing which to watch.
function describe(view: TerminalView): string {
  const who = view.opened_by === "agent" ? "Your agent" : "You";
  const what = [view.command, ...(view.args ?? [])].join(" ");
  return view.exit ? `${who} ran ${what} — ${view.exit}` : `${who} started ${what}`;
}

export function TerminalPanel({ hidden }: { hidden: boolean }) {
  const state = useSession();
  useXtermStyle();
  const host = useRef<HTMLDivElement>(null);
  const screen = useRef<Terminal | null>(null);
  const fit = useRef<FitAddon | null>(null);
  const [open, setOpen] = useState<TerminalView | null>(null);
  const [others, setOthers] = useState<TerminalView[]>([]);
  const [status, setStatus] = useState("");
  const running = useRef(0);
  // A terminal nobody is looking at does not stream. Its output is buffered
  // on the host with a sequence to continue from, so nothing is lost by
  // waiting - and a browser keeps only a handful of connections per origin,
  // so a held-open poll behind a hidden panel starves the page that is
  // actually in use (it stopped a composition from loading its audio at all).
  const watching = useRef(!hidden);
  const inFlight = useRef<AbortController | null>(null);
  useEffect(() => {
    watching.current = !hidden;
    // Let go of the connection at once rather than when the held-open request
    // happens to return: twenty seconds of holding one is long enough to stop
    // the page a person actually switched to.
    if (hidden) inFlight.current?.abort();
  }, [hidden]);

  // Every terminal this profile has open, not only the one this panel
  // started. An agent working alone opens its own to run a command, and a
  // person who cannot see it cannot watch what their agent is doing.
  useEffect(() => {
    if (hidden || !state.profileId) return;
    let live = true;
    const read = async () => {
      try {
        const answer = await sessionStore.api<{ terminals: TerminalView[] }>("GET", "/api/terminals");
        if (live) setOthers(answer.terminals.filter((one) => one.profile_id === state.profileId));
      } catch {
        // The list is how a person finds a terminal, not how they use one.
      }
    };
    void read();
    const timer = setInterval(() => void read(), 2000);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [hidden, state.profileId]);

  // One terminal at a time, owned outside React's render: xterm keeps the
  // screen, so re-rendering must not recreate it.
  useEffect(() => {
    if (hidden || host.current === null || screen.current !== null) return;
    // xterm paints with values, not names, so the values are read from the
    // shell's own palette rather than picked here.
    const palette = getComputedStyle(document.documentElement);
    const token = (name: string) => palette.getPropertyValue(name).trim() || undefined;
    const terminal = new Terminal({
      convertEol: false,
      cursorBlink: true,
      fontSize: 13,
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
      theme: {
        background: token("--bg"),
        foreground: token("--text"),
        cursor: token("--accent"),
        selectionBackground: token("--panel"),
      },
    });
    const addon = new FitAddon();
    terminal.loadAddon(addon);
    terminal.open(host.current);
    addon.fit();
    screen.current = terminal;
    fit.current = addon;
  }, [hidden]);

  useEffect(() => {
    if (hidden) return;
    fit.current?.fit();
  }, [hidden, open]);

  const start = async () => {
    if (!state.profileId) {
      setStatus("Choose an agent first.");
      return;
    }
    setStatus("Starting…");
    try {
      const terminal = screen.current;
      const view = await sessionStore.api<TerminalView>("POST", "/api/terminals", {
        profile_id: state.profileId,
        cols: terminal?.cols ?? 80,
        rows: terminal?.rows ?? 24,
      });
      terminal?.clear();
      terminal?.focus();
      setOpen(view);
      setStatus("");
      void pump(view.terminal_id, (running.current += 1));
    } catch (error) {
      setStatus((error as Error).message);
    }
  };

  // Watch a terminal this panel did not open. Reading starts from the
  // beginning, so what the agent's command has already said is on the screen
  // rather than only what it says next.
  const watch = (view: TerminalView) => {
    screen.current?.clear();
    screen.current?.focus();
    setOpen(view);
    setStatus(view.exit ?? "");
    void pump(view.terminal_id, (running.current += 1));
  };

  /// Read output until this terminal is replaced or ends.
  const pump = async (terminalId: string, generation: number) => {
    let after = 0;
    while (running.current === generation) {
      if (!watching.current) {
        await new Promise((resolve) => setTimeout(resolve, 250));
        continue;
      }
      const controller = new AbortController();
      inFlight.current = controller;
      try {
        const answer = await sessionStore.api<{ next: number; bytes: string; dropped: boolean; exit?: string }>(
          "GET",
          `/api/terminals/${encodeURIComponent(terminalId)}/output?after=${after}&wait_ms=20000`,
          undefined,
          controller.signal,
        );
        if (running.current !== generation) return;
        if (answer.dropped) screen.current?.write("\r\n[earlier output dropped]\r\n");
        if (answer.bytes) screen.current?.write(decode(answer.bytes));
        after = answer.next;
        if (answer.exit) {
          setOpen((current) => (current ? { ...current, exit: answer.exit } : current));
          setStatus(answer.exit);
          return;
        }
      } catch (error) {
        if (running.current !== generation) return;
        // An abort is this panel stepping aside, not a failure.
        if ((error as Error).name === "AbortError" || !watching.current) continue;
        setStatus((error as Error).message);
        return;
      } finally {
        if (inFlight.current === controller) inFlight.current = null;
      }
    }
  };

  // What a person types goes to the process as exact bytes.
  useEffect(() => {
    const terminal = screen.current;
    if (!terminal || open === null) return;
    const typed = terminal.onData((data) => {
      void sessionStore
        .api("POST", `/api/terminals/${encodeURIComponent(open.terminal_id)}/input`, { bytes: encode(data) })
        .catch((error: Error) => setStatus(error.message));
    });
    const resized = terminal.onResize(({ cols, rows }) => {
      void sessionStore
        .api("POST", `/api/terminals/${encodeURIComponent(open.terminal_id)}/size`, { cols, rows })
        .catch(() => undefined);
    });
    return () => {
      typed.dispose();
      resized.dispose();
    };
  }, [open?.terminal_id]);

  // Step away from a terminal without ending it. Watching one the agent
  // started is looking, not owning: the only way out used to be Close, which
  // kills the command the agent is in the middle of.
  const lookAway = () => {
    running.current += 1;
    screen.current?.clear();
    setOpen(null);
    setStatus("");
  };

  const stop = async () => {
    if (open === null) return;
    running.current += 1;
    try {
      await sessionStore.api("DELETE", `/api/terminals/${encodeURIComponent(open.terminal_id)}`);
    } catch {
      // A terminal that already ended is already closed.
    }
    setOpen(null);
    setStatus("");
  };

  return (
    <section className="terminal-panel" data-agent-panel="terminal" hidden={hidden} aria-label="Terminal">
      <div className="terminal-actions">
        {/* A terminal of their own, whatever else is on the screen. This was
            disabled while one was running, which read as "start another" and
            did nothing - and after watching the agent's, the only way to get
            a prompt back was to close the agent's command. */}
        <button id="terminal-open" className="primary" onClick={() => void start()}>
          {open === null ? "Open a terminal" : "Start another"}
        </button>
        {open?.opened_by === "agent" ? (
          <button id="terminal-detach" onClick={() => lookAway()}>
            Stop watching
          </button>
        ) : null}
        <button id="terminal-close" disabled={open === null} onClick={() => void stop()}>
          {open?.opened_by === "agent" ? "End the agent's command" : "Close"}
        </button>
        <span id="terminal-id" className="k-caption k-muted">
          {open?.terminal_id ?? ""}
        </span>
        <span id="terminal-status" className="k-caption">
          {status}
        </span>
      </div>
      {others.length > 0 && (
        <ul className="terminal-list" aria-label="Open terminals">
          {others.map((one) => (
            <li
              key={one.terminal_id}
              data-terminal-id={one.terminal_id}
              data-opened-by={one.opened_by}
              aria-current={one.terminal_id === open?.terminal_id}
            >
              <button
                className="terminal-watch"
                disabled={one.terminal_id === open?.terminal_id}
                onClick={() => watch(one)}
              >
                Watch
              </button>
              <span className="terminal-what">{describe(one)}</span>
            </li>
          ))}
        </ul>
      )}
      <p className="k-caption k-muted">
        This runs in the agent's environment: its working directory, its home, and the keys you
        gave the profile. Sign an agent in here and it stays signed in.
      </p>
      {open === null ? (
        // An empty black rectangle says nothing about what it is for. The
        // screen stays mounted (xterm owns it), so the line sits above it.
        <p id="terminal-none" className="k-caption k-muted">
          Nothing open here yet. Open a terminal, or watch one your agent started.
        </p>
      ) : null}
      <div id="terminal-screen" className="terminal-screen" ref={host} />
    </section>
  );
}
