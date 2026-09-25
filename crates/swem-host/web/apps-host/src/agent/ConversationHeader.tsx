// The strip over the conversation: which model the session runs on, which
// mode it is in, what the agent has used of its window, and the plan it
// wrote for the turn. All of it is the agent's own, read from the session,
// never invented here: an agent that offers no model choice shows none.

import { useSession } from "./store.ts";
import { sessionStore } from "./store.ts";

export function ConversationHeader({ hidden }: { hidden: boolean }) {
  const { conversation, sessionOptions, connectionId, routeId } = useSession();
  const connected = connectionId !== null && routeId !== null;
  const options = sessionOptions?.options ?? [];
  const model = options.find((option) => option.category === "model" && option.kind === "select") ?? null;
  const modeOption = options.find((option) => option.category === "mode" && option.kind === "select") ?? null;
  const legacyModes = sessionOptions?.modes ?? null;
  // The mode's name, whichever road the agent offers it on; the id when the
  // agent gave no name.
  const modeChoices = modeOption?.choices ?? legacyModes?.available ?? [];
  const currentMode = modeOption ? String(modeOption.currentValue) : (legacyModes?.current ?? conversation.mode ?? "");
  const modeName = modeChoices.find((choice) => choice.value === currentMode)?.name ?? currentMode;
  const nextMode = () => {
    if (modeChoices.length === 0) return;
    const index = modeChoices.findIndex((choice) => choice.value === currentMode);
    const next = modeChoices[(index + 1) % modeChoices.length];
    if (!next) return;
    if (modeOption) void sessionStore.setSessionOption(modeOption.id, next.value);
    else void sessionStore.setSessionMode(next.value);
  };
  const usage = conversation.usage;
  const plan = conversation.plan;
  const done = plan.filter((entry) => entry.status === "completed").length;
  return (
    <div className="conversation-header" hidden={hidden}>
      <label className="conversation-model">
        <span className="k-caption">Model</span>
        <select
          id="session-model"
          aria-label="Model"
          disabled={!connected || !model}
          value={model ? String(model.currentValue) : ""}
          onChange={(event) => model && void sessionStore.setSessionOption(model.id, event.target.value)}
        >
          {model ? (
            model.choices.map((choice) => (
              <option value={choice.value} key={choice.value}>
                {choice.name}
              </option>
            ))
          ) : (
            <option value="">agent's default</option>
          )}
        </select>
      </label>
      {modeChoices.length > 0 || currentMode ? (
        <button id="session-mode" className="k-chip session-mode" disabled={!connected || modeChoices.length < 2} title="Switch mode" onClick={nextMode}>
          {modeName || "mode"}
        </button>
      ) : null}
      {usage && (usage.used !== null || usage.cost !== null) ? (
        <span id="session-usage" className="k-caption session-usage">
          {usage.used !== null && usage.size !== null ? `${usage.used.toLocaleString()} / ${usage.size.toLocaleString()} tokens` : ""}
          {usage.cost !== null ? ` · ${usage.cost} ${usage.currency ?? ""}`.trimEnd() : ""}
        </span>
      ) : null}
      {plan.length > 0 ? (
        <details id="plan-toggle" className="session-plan">
          <summary className="k-caption">
            Plan · {done}/{plan.length}
          </summary>
          <ol id="plan">
            {plan.map((entry, index) => (
              <li data-status={entry.status} data-priority={entry.priority ?? undefined} key={index}>
                {entry.content}
              </li>
            ))}
          </ol>
        </details>
      ) : null}
    </div>
  );
}
