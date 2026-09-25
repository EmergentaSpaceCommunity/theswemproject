import { useSession } from "../agent/store.ts";

/// The Agent space's live connection, for the Project space to act through.
export function useConnectionId(): string | null {
  return useSession().connectionId;
}
