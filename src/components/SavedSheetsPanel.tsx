import { useState } from "react";
import type { SavedSheet } from "../lib/api";
import { useI18n } from "../lib/i18n";

interface Props {
  /** "save" asks for a name, "open" lists what the app already keeps. */
  mode: "open" | "save";
  sheets: SavedSheet[];
  defaultName: string;
  busy: boolean;
  onClose: () => void;
  onSave: (name: string) => void;
  onOpen: (sheet: SavedSheet) => void;
  onDelete: (sheet: SavedSheet) => void;
}

/**
 * Sheets are stored in the library database, so this list replaces the file
 * picker: the layout and its copies come back with a click, nothing to move.
 */
export function SavedSheetsPanel({
  mode,
  sheets,
  defaultName,
  busy,
  onClose,
  onSave,
  onOpen,
  onDelete,
}: Props) {
  const { t, locale } = useI18n();
  const [name, setName] = useState(defaultName);

  const when = (ms: number) =>
    new Date(ms).toLocaleString(locale === "es" ? "es-ES" : "en-GB", {
      day: "2-digit",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    });

  return (
    <div className="picker-backdrop" onPointerDown={() => !busy && onClose()}>
      <div
        className="picker sheets-panel"
        role="dialog"
        aria-modal="true"
        aria-label={t("sheet.sheetsTitle")}
        onPointerDown={(event) => event.stopPropagation()}
      >
        <header className="picker-head">
          <div className="series-headtext">
            <span className="field-label">{t("sheet.sheetsTitle")}</span>
            <p className="muted small series-note">{t("sheet.sheetsNote")}</p>
          </div>
          <button
            className="icon-btn"
            onClick={onClose}
            disabled={busy}
            aria-label={t("detail.close")}
          >
            ✕
          </button>
        </header>

        {mode === "save" && (
          <label className="sheet-field sheets-name">
            <span className="field-label">{t("sheet.sheetName")}</span>
            <input
              className="text-input"
              value={name}
              placeholder={t("sheet.sheetNamePlaceholder")}
              autoFocus
              disabled={busy}
              onChange={(event) => setName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !busy) onSave(name);
              }}
            />
          </label>
        )}

        <div className="picker-list">
          {sheets.length === 0 && (
            <p className="muted small copy-empty">{t("sheet.sheetsEmpty")}</p>
          )}
          {sheets.map((sheet) => (
            <div className="saved-sheet-row" key={sheet.id}>
              <div className="archive-names">
                <span className="archive-name" title={sheet.name}>
                  {sheet.name}
                </span>
                <span className="archive-existing">
                  {t("sheet.sheetCopies", { n: sheet.copies })} · {when(sheet.updatedAt)}
                </span>
              </div>
              {mode === "open" && (
                <button className="ghost-btn small-btn" onClick={() => onOpen(sheet)} disabled={busy}>
                  {t("sheet.sheetOpen")}
                </button>
              )}
              <button
                className="icon-btn"
                onClick={() => onDelete(sheet)}
                disabled={busy}
                title={t("sheet.sheetDelete")}
                aria-label={`${t("sheet.sheetDelete")}: ${sheet.name}`}
              >
                ✕
              </button>
            </div>
          ))}
        </div>

        <footer className="picker-foot">
          <span className="muted small">
            {busy ? t("sheet.sheetsSaving") : ""}
          </span>
          <div className="archive-actions">
            <button className="ghost-btn" onClick={onClose} disabled={busy}>
              {t("common.cancel")}
            </button>
            {mode === "save" && (
              <button className="primary-btn" onClick={() => onSave(name)} disabled={busy}>
                {t("sheet.saveLocal")}
              </button>
            )}
          </div>
        </footer>
      </div>
    </div>
  );
}
