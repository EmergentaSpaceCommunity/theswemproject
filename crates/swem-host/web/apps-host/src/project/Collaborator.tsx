import { RefChip } from "./RefChip";
import type { AgentContextBinding, Viewing } from "./types";

interface Props {
  viewing: Viewing;
  connectionId: string | null;
  context: AgentContextBinding | null;
  contextError: string | null;
  bindable: boolean;
  onBind: () => void;
  onClear: () => void;
}

/// Two anchors that never merge: what the operator views and what the agent
/// was explicitly handed. Only the bind and clear buttons write the latter.
export function Collaborator({ viewing, connectionId, context, contextError, bindable, onBind, onClear }: Props) {
  return (
    <aside id="project-collaborator" className="context" aria-label="Collaborator">
      <h2>Collaborator</h2>
      <div className="k-eyebrow">Viewing</div>
      <div
        id="viewing"
        data-server-name={viewing.server_name ?? ""}
        data-logical-id={viewing.logical_id ?? ""}
        data-revision-ref={viewing.revision_ref ?? ""}
        data-selection-ref={viewing.selection_ref ?? ""}
      >
        {viewing.revision_ref ? (
          <>
            <div>
              revision <RefChip value={viewing.revision_ref} />
            </div>
            {viewing.selection_ref ? (
              <div>
                branch <RefChip value={viewing.selection_ref} />
              </div>
            ) : null}
          </>
        ) : (
          <span className="k-muted">nothing selected</span>
        )}
      </div>
      <div className="k-eyebrow">Agent context</div>
      <div
        id="agent-context"
        data-connection-id={connectionId ?? ""}
        data-server-name={context?.server_name ?? ""}
        data-revision-ref={context?.revision_ref ?? ""}
        data-selection-ref={context?.selection_ref ?? ""}
        data-uri={context?.uri ?? ""}
      >
        {!connectionId ? (
          <span className="k-muted">no agent session open - start one in the Agent space</span>
        ) : context ? (
          <>
            <div>
              revision <RefChip value={context.revision_ref} />
            </div>
            {context.selection_ref ? (
              <div>
                branch <RefChip value={context.selection_ref} />
              </div>
            ) : null}
            <small className="k-muted">sent with every next turn as a resource link</small>
          </>
        ) : (
          <span className="k-muted">none bound</span>
        )}
      </div>
      <div className="session-actions" style={{ marginTop: 10 }}>
        <button id="bind-project" className="primary" disabled={!bindable} onClick={onBind}>
          Work with this project
        </button>
        <button id="clear-context" disabled={!connectionId || !context} onClick={onClear}>
          Clear context
        </button>
      </div>
      <div id="bind-status" className="bad">
        {contextError ?? ""}
      </div>
      <div className="k-eyebrow">Scope</div>
      <div id="project-scope" className="k-muted">
        independent host connection · reads only · records are the server's
      </div>
    </aside>
  );
}
