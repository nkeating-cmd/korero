// Kōrero (v1.25.0, UX batch): shared confirmation for destructive actions.
// One misdirected click could previously delete a note, history entry,
// meeting, or recording irreversibly (UX audit finding #2). Uses a sonner
// toast with an explicit Delete action rather than a blocking modal — matches
// the app's existing toast-action pattern (teach-a-correction) and keeps the
// flow one-keystroke cancellable (toast auto-dismisses = implicit cancel).
import { toast } from "sonner";

export function confirmDestructive(
  message: string,
  description: string | undefined,
  actionLabel: string,
  onConfirm: () => void | Promise<void>,
) {
  toast(message, {
    description,
    duration: 8000,
    action: {
      label: actionLabel,
      onClick: () => {
        void onConfirm();
      },
    },
    cancel: {
      label: "Keep",
      onClick: () => {
        /* dismiss */
      },
    },
  });
}
