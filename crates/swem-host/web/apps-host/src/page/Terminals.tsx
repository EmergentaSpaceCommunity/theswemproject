// An agent's terminals: what is open where it lives, each by what runs in
// it and who started it, and one of them on the screen.
//
// Agents are signed in at a prompt: a device code, a browser handshake, a
// `login` subcommand. This runs a real terminal where the agent lives, in
// the folder it works in and with what it is given, so what is signed in
// here is signed in for the agent.
//
// One screen. The terminal that is chosen is read from its beginning,
// which the host keeps, so choosing another loses nothing. Nothing here is
// recorded: a secret typed at a prompt must not become a record.

import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import xtermCss from "@xterm/xterm/css/xterm.css?raw";
import { useEffect, useRef, useState } from "react";

import type { TerminalView } from "../agent/session.ts";
import { sessionStore, useSession } from "../agent/store.ts";
import { Cross, Plus, Prompt } from "./icons.tsx";
import { running } from "./terminals.ts";
import type { Participant } from "./types.ts";

/// xterm's own stylesheet, inlined once: the page is a single script.
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

/// xterm paints with values, not names: they are read from the palette as
/// it is now.
function colours() {
  const palette = getComputedStyle(document.documentElement);
  const value = (name: string) => palette.getPropertyValue(name).trim() || undefined;
  return {
    background: value("--color-background-primary"),
    foreground: value("--color-text-primary"),
    cursor: value("--color-text-info"),
    selectionBackground: value("--color-background-tertiary"),
  };
}

export function Terminals({ agent, hidden }: { agent: Participant; hidden: boolean }) {
  const state = useSession();
  useXtermStyle();
  const host = useRef<HTMLDivElement>(null);
  const screen = useRef<Terminal | null>(null);
  const fit = useRef<FitAddon | null>(null);
  const [open, setOpen] = useState<TerminalView | null>(null);
  const [others, setOthers] = useState<TerminalView[]>([]);
  const [status, setStatus] = useState("");
  const running_ = useRef(0);
  const profile = agent.profile_id ?? "";
  const inAContainer = state.profiles.find((one) => one.profile_id === profile)?.environment_profile_id === "podman-container-environment";
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
    if (hidden || !profile) return;
    let live = true;
    const read = async () => {
      try {
        const answer = await sessionStore.api<{ terminals: TerminalView[] }>("GET", "/api/terminals");
        if (live) setOthers(answer.terminals.filter((one) => one.profile_id === profile));
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
  }, [hidden, profile]);

  // One terminal at a time, owned outside React's render: xterm keeps the
  // screen, so re-rendering must not recreate it.
  useEffect(() => {
    if (hidden || host.current === null || screen.current !== null) return;
    const terminal = new Terminal({
      convertEol: false,
      cursorBlink: true,
      fontSize: 13,
      fontFamily: getComputedStyle(document.documentElement).getPropertyValue("--font-mono").trim() || "ui-monospace, monospace",
      theme: colours(),
    });
    const addon = new FitAddon();
    terminal.loadAddon(addon);
    terminal.open(host.current);
    addon.fit();
    screen.current = terminal;
    fit.current = addon;
  }, [hidden]);

  // The screen follows the theme, and the room it is given.
  useEffect(() => {
    if (hidden || host.current === null) return undefined;
    const themed = new MutationObserver(() => {
      if (screen.current) screen.current.options.theme = colours();
    });
    themed.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    const sized = new ResizeObserver(() => fit.current?.fit());
    sized.observe(host.current);
    return () => {
      themed.disconnect();
      sized.disconnect();
    };
  }, [hidden]);

  useEffect(() => {
    if (hidden) return;
    fit.current?.fit();
  }, [hidden, open]);

  const start = async () => {
    if (!profile) return;
    setStatus("Starting…");
    try {
      const terminal = screen.current;
      const view = await sessionStore.api<TerminalView>("POST", "/api/terminals", {
        profile_id: profile,
        cols: terminal?.cols ?? 80,
        rows: terminal?.rows ?? 24,
      });
      terminal?.clear();
      terminal?.focus();
      setOpen(view);
      setStatus("");
      void pump(view.terminal_id, (running_.current += 1));
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
    void pump(view.terminal_id, (running_.current += 1));
  };

  /// Read output until this terminal is replaced or ends.
  const pump = async (terminalId: string, generation: number) => {
    let after = 0;
    while (running_.current === generation) {
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
        if (running_.current !== generation) return;
        if (answer.dropped) screen.current?.write("\r\n[earlier output dropped]\r\n");
        if (answer.bytes) screen.current?.write(decode(answer.bytes));
        after = answer.next;
        if (answer.exit) {
          setOpen((current) => (current ? { ...current, exit: answer.exit } : current));
          setStatus(answer.exit);
          return;
        }
      } catch (error) {
        if (running_.current !== generation) return;
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

  const stop = async () => {
    if (open === null) return;
    running_.current += 1;
    try {
      await sessionStore.api("DELETE", `/api/terminals/${encodeURIComponent(open.terminal_id)}`);
    } catch {
      // A terminal that already ended is already closed.
    }
    setOpen(null);
    setStatus("");
  };

  /// Close a terminal, or end what the agent runs in one.
  const end = async (view: TerminalView) => {
    if (view.terminal_id === open?.terminal_id) {
      await stop();
      return;
    }
    try {
      await sessionStore.api("DELETE", `/api/terminals/${encodeURIComponent(view.terminal_id)}`);
    } catch {
      // A terminal that already ended is already closed.
    }
  };

  return (
    <section className="w-terminal" data-agent-panel="terminal" hidden={hidden} aria-label={`Terminals of ${agent.name}`}>
      <div className="k-spread w-terminal-head">
        <div className="w-terms" role="tablist" aria-label="Open terminals">
          {others.map((one) => {
            const shown = one.terminal_id === open?.terminal_id;
            const theirs = one.opened_by === "agent";
            return (
              <span className={`w-term${shown ? " k-active" : ""}`} key={one.terminal_id}>
                <button type="button" role="tab" aria-selected={shown} className="w-term-name" onClick={() => (shown ? undefined : watch(one))}>
                  <Prompt size={15} />
                  <span className="k-two">
                    <span className="k-name k-mono">{running(one)}</span>
                    <span className="k-caption">{[theirs ? `started by ${agent.name}` : "yours", one.exit ? "ended" : ""].filter(Boolean).join(" · ")}</span>
                  </span>
                </button>
                <button
                  type="button"
                  className="k-btn k-quiet"
                  title={theirs && !one.exit ? `End what ${agent.name} runs here` : "Close"}
                  aria-label={theirs && !one.exit ? `End what ${agent.name} runs in ${running(one)}` : `Close ${running(one)}`}
                  onClick={() => void end(one)}
                >
                  <Cross size={13} />
                </button>
              </span>
            );
          })}
        </div>
        <button type="button" className="k-btn k-primary" onClick={() => void start()}>
          <Plus size={15} />
          <span>New terminal</span>
        </button>
      </div>
      <div className="k-caption w-terminal-says" role="status">
        {status ||
          (inAContainer
            ? `On this machine, beside ${agent.name}'s container: in its folder and with its home.`
            : `On this machine, in ${agent.name}'s folder, with its keys and its sign-in.`)}
      </div>
      {open === null ? (
        // The screen stays mounted, because xterm owns it; what it is for is
        // said over it while nothing is on it.
        <div className="k-caption w-terminal-none">
          {others.length === 0 ? "Nothing is open here. A new terminal is a shell of yours where the agent lives." : "Choose a terminal to see what it says."}
        </div>
      ) : null}
      <div className="w-terminal-screen" ref={host} hidden={open === null} />
    </section>
  );
}
