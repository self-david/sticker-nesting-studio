import { useEffect } from "react";
import { useI18n } from "../lib/i18n";

interface Props {
  message: string;
  onAnswer: (value: boolean) => void;
}

/**
 * The app's own question. The webview's `confirm` hands control back before
 * the user answers, so a window.confirm let actions run without a yes.
 */
export function ConfirmDialog({ message, onAnswer }: Props) {
  const { t } = useI18n();

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      onAnswer(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onAnswer]);

  return (
    <div className="picker-backdrop" onPointerDown={() => onAnswer(false)}>
      <div
        className="picker ask-panel"
        role="alertdialog"
        aria-modal="true"
        onPointerDown={(event) => event.stopPropagation()}
      >
        <p className="ask-text">{message}</p>
        <div className="archive-actions ask-actions">
          <button className="ghost-btn" autoFocus onClick={() => onAnswer(false)}>
            {t("common.cancel")}
          </button>
          <button className="primary-btn" onClick={() => onAnswer(true)}>
            {t("common.yes")}
          </button>
        </div>
      </div>
    </div>
  );
}
