// What the product says about the last thing a person did in an agent's
// settings, said beside the part they did it in.

import { useStore } from "zustand";
import { createStore } from "zustand/vanilla";

import { useSession } from "../../agent/store.ts";

export type Part = "together" | "identity" | "place" | "tools" | "skills" | "permissions" | "signin" | "remove";

const last = createStore<{ part: Part | null }>(() => ({ part: null }));

/// What is done next belongs to this part.
export const inPart = (part: Part): void => last.setState({ part });

export function Said({ part }: { part: Part }) {
  const spoke = useStore(last, (state) => state.part);
  const { environmentStatus } = useSession();
  if (spoke !== part || !environmentStatus) return null;
  const well = environmentStatus === "Saved." || environmentStatus === "Saving…";
  return (
    <span id="environment-status" role="status" className={`k-caption${well ? "" : " k-is-danger"}`}>
      {environmentStatus}
    </span>
  );
}
