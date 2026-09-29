// What is fetched and from where, asked before it is installed.

import { Dialog } from "@base-ui/react/dialog";

/// What is fetched and from where, asked before it is installed.
export function Consent({ asked, onAnswer }: { asked: string | null; onAnswer: (yes: boolean) => void }) {
  const [title, ...rest] = (asked ?? "").split("\n\n");
  return (
    <Dialog.Root open={asked !== null} onOpenChange={(next) => (next ? undefined : onAnswer(false))}>
      <Dialog.Portal>
        <Dialog.Backdrop className="w-scrim" />
        <Dialog.Popup className="k-dialog w-dialog">
          <Dialog.Title className="k-heading">{title}</Dialog.Title>
          <Dialog.Description className="k-caption">{rest.join(" ")}</Dialog.Description>
          <div className="k-inline w-end">
            <button type="button" className="k-btn k-quiet" onClick={() => onAnswer(false)}>
              Not now
            </button>
            <button type="button" className="k-btn k-primary" onClick={() => onAnswer(true)}>
              Install
            </button>
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
