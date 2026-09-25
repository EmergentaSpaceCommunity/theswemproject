// An ACP permission request, answered by one click on one of its options.

import type { PermissionProvenance } from "./session.ts";
import { sessionStore, useSession } from "./store.ts";

/// Whose words the person is reading. Two questions can carry the same
/// sentence and mean different things: an agent saying what it intends to do
/// is a report, and the host saying what it is about to run is a fact about
/// the next thing that happens. A person answering cannot tell them apart
/// from the sentence, so the dialog says which it is.
function whose(provenance: PermissionProvenance | undefined): string {
  if (provenance === "host_callback") return "This exact command runs here if you allow it.";
  if (provenance === "uncorrelated_agent_report") {
    return "Your agent's own words. It has not reported the step this belongs to.";
  }
  return "Your agent's own words, for a step it has reported.";
}

/// What an option does, in the person's words rather than the wire's.
///
/// The name on the button is the agent's own text and the kind is the wire's
/// meaning, so the two are shown together: an option called "Yes" whose kind
/// refuses from now on is something a person has to be able to see. An
/// unknown kind is shown as it came, because hiding it would hide exactly the
/// case this line is for.
function kindWords(kind: string): string {
  if (kind === "allow_once") return "allows once";
  if (kind === "allow_always") return "allows from now on";
  if (kind === "reject_once") return "refuses once";
  if (kind === "reject_always") return "refuses from now on";
  return kind;
}

export function PermissionDialog() {
  const { permission } = useSession();
  // The question a person reads is the question, and nothing else. The
  // sequence is how this host tells two pending questions apart; it belongs
  // to the dialog as data, not to the sentence somebody has to answer.
  const title = permission?.tool_call.title ?? permission?.tool_call.toolCallId ?? "";
  return (
    <div
      id="permission"
      data-sequence={permission?.sequence ?? ""}
      data-provenance={permission?.provenance ?? ""}
      style={{ display: permission ? "grid" : "none" }}
    >
      <div className="permission-card k-dialog" role="dialog" aria-modal="true" aria-label="Permission required">
        <div className="k-eyebrow">Permission required</div>
        <div id="permission-title">{title}</div>
        <div id="permission-whose" className="k-caption k-muted">
          {permission ? whose(permission.provenance) : ""}
        </div>
        <div id="permission-options">
          {permission?.options.map((option) => (
            <button
              className="permission-option"
              data-option-id={option.optionId}
              data-kind={option.kind}
              key={option.optionId}
              onClick={() => void sessionStore.selectPermission(option.optionId)}
            >
              {option.name}
              <span className="k-caption permission-kind"> — {kindWords(option.kind)}</span>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
