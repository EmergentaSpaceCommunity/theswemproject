// What is said of a terminal on the page.

import type { TerminalView } from "../agent/session.ts";

/// What runs in a terminal, as a person would say it: the program by its
/// name, and what it was given.
export function running(view: Pick<TerminalView, "command" | "args">): string {
  const program = view.command.split("/").pop() ?? view.command;
  // A shell given one line to run is that line.
  const given = view.args ?? [];
  return given[0] === "-c" || given[0] === "-lc" ? given.slice(1).join(" ") : [program, ...given].join(" ");
}
