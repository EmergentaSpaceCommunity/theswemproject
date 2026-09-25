import { RefChip } from "./RefChip";
import type { LineView, OperationView } from "./types";

/// The activity drawer: the project's long operations (as their start and
/// end records tell them), then the viewed line's amendments and authority
/// decisions, as recorded - no timestamps exist in the records, so none are
/// shown.
export function Activity({ line, operations }: { line: LineView | null; operations: OperationView[] }) {
  return (
    <footer id="project-activity">
      <div className="k-eyebrow">Activity</div>
      {operations.map((operation) => (
        <div
          key={operation.id}
          className="activity-row k-row operation"
          data-operation-id={operation.id}
          data-status={operation.status}
          data-tool={operation.tool}
        >
          <span className="operation-status">{operation.status}</span> {operation.tool || "operation"}{" "}
          {operation.delivery_selection_ref ? (
            <>
              over <RefChip value={operation.delivery_selection_ref} />
            </>
          ) : null}
          {Object.entries(operation.result_refs).map(([field, ref]) => (
            <span key={field} className="operation-result" data-field={field}>
              {" "}
              {field}: <RefChip value={ref} />
            </span>
          ))}
          {operation.error ? <span className="operation-error"> {operation.error}</span> : null}
        </div>
      ))}
      {!line ? <span className="k-muted">select a revision line</span> : null}
      {line?.amendments.map((amendment) => (
        <div key={amendment.ref} className="activity-row k-row amendment" data-ref={amendment.ref}>
          <RefChip value={amendment.ref} /> amendment by {amendment.introduced_by}: <RefChip value={amendment.parent_ref} /> →{" "}
          <RefChip value={amendment.child_ref} />
        </div>
      ))}
      {line?.amendment_decisions.map((decision) => (
        <div key={decision.ref} className="activity-row k-row decision" data-ref={decision.ref} data-allowed={decision.allowed ? "true" : "false"}>
          <RefChip value={decision.ref} /> {decision.allowed ? "allowed" : "refused"} for {decision.authority_ref}
        </div>
      ))}
    </footer>
  );
}
