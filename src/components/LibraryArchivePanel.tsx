import { useMemo, useState } from "react";
import type { ArchiveAction, ArchiveConflict } from "../lib/api";
import { useI18n } from "../lib/i18n";

interface Props {
  total: number;
  conflicts: ArchiveConflict[];
  busy: boolean;
  onClose: () => void;
  onImport: (decisions: { id: string; action: ArchiveAction }[]) => void;
}

/**
 * The archive brought stickers whose id is already here. Each one can keep this
 * library's copy, take the archive's, or be stored twice (the new one under a
 * fresh id), and the whole list can be set at once.
 */
export function LibraryArchivePanel({ total, conflicts, busy, onClose, onImport }: Props) {
  const { t } = useI18n();
  const [choices, setChoices] = useState<Record<string, ArchiveAction>>({});

  const actionOf = (id: string): ArchiveAction => choices[id] ?? "keep";

  const tally = useMemo(() => {
    let replaced = 0;
    let both = 0;
    let keep = 0;
    for (const conflict of conflicts) {
      const action = choices[conflict.id] ?? "keep";
      if (action === "replace") replaced += 1;
      else if (action === "both") both += 1;
      else keep += 1;
    }
    return { fresh: total - conflicts.length, replaced, both, keep };
  }, [conflicts, choices, total]);

  const setAll = (action: ArchiveAction) =>
    setChoices(Object.fromEntries(conflicts.map((conflict) => [conflict.id, action])));

  return (
    <div className="picker-backdrop" onPointerDown={() => !busy && onClose()}>
      <div
        className="picker archive-panel"
        role="dialog"
        aria-modal="true"
        aria-label={t("archive.title")}
        onPointerDown={(event) => event.stopPropagation()}
      >
        <header className="picker-head">
          <div className="series-headtext">
            <span className="field-label">{t("archive.title")}</span>
            <p className="muted small series-note">
              {t("archive.note", { total, n: conflicts.length })}
            </p>
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

        <div className="archive-tools">
          <span>{t("archive.all")}</span>
          <button className="link-btn" onClick={() => setAll("keep")}>
            {t("archive.keep")}
          </button>
          <button className="link-btn" onClick={() => setAll("replace")}>
            {t("archive.replace")}
          </button>
          <button className="link-btn" onClick={() => setAll("both")}>
            {t("archive.both")}
          </button>
        </div>

        <div className="picker-list">
          {conflicts.map((conflict) => (
            <div className="archive-row" key={conflict.id}>
              <div className="archive-names">
                <span className="archive-name" title={conflict.name}>
                  {conflict.name}
                </span>
                <span className="archive-existing" title={conflict.existingName}>
                  {t("archive.existingName", { name: conflict.existingName })}
                </span>
              </div>
              <select
                className="archive-choice"
                value={actionOf(conflict.id)}
                disabled={busy}
                aria-label={`${conflict.name}: ${t("archive.title")}`}
                onChange={(event) =>
                  setChoices((prev) => ({
                    ...prev,
                    [conflict.id]: event.target.value as ArchiveAction,
                  }))
                }
              >
                <option value="keep">{t("archive.keep")}</option>
                <option value="replace">{t("archive.replace")}</option>
                <option value="both">{t("archive.both")}</option>
              </select>
            </div>
          ))}
        </div>

        <footer className="picker-foot">
          <span className="muted small">
            {busy
              ? t("archive.reading")
              : t("archive.summary", {
                  fresh: tally.fresh,
                  replaced: tally.replaced,
                  both: tally.both,
                  keep: tally.keep,
                })}
          </span>
          <div className="archive-actions">
            <button className="ghost-btn" onClick={onClose} disabled={busy}>
              {t("common.cancel")}
            </button>
            <button
              className="primary-btn"
              disabled={busy}
              onClick={() =>
                onImport(
                  conflicts.map((conflict) => ({
                    id: conflict.id,
                    action: actionOf(conflict.id),
                  })),
                )
              }
            >
              {t("archive.import")}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}
