import { useState } from "react";
import type { MergeGroup } from "../types";
import { useI18n, type MessageKey } from "../lib/i18n";

interface Props {
  /** The text-only pass; it throws with the message to show. */
  scan: () => Promise<MergeGroup[]>;
  /** Applies one group and returns the note to show, or null. */
  apply: (group: MergeGroup) => Promise<string | null>;
  keys: {
    title: MessageKey;
    note: MessageKey;
    scan: MessageKey;
    scanning: MessageKey;
    none: MessageKey;
    apply: MessageKey;
    applying: MessageKey;
    failed: MessageKey;
  };
  disabled?: boolean;
}

/**
 * The proposal list shared by tags and series: the model suggests groups, one
 * is applied at a time, and scanning alone writes nothing.
 */
export function MergeSuggestions({ scan, apply, keys, disabled = false }: Props) {
  const { t } = useI18n();
  const [groups, setGroups] = useState<MergeGroup[] | null>(null);
  const [scanning, setScanning] = useState(false);
  const [applying, setApplying] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setScanning(true);
    setNote(null);
    setError(null);
    setGroups(null);
    try {
      setGroups(await scan());
    } catch (err) {
      setError(t(keys.failed, { message: String(err) }));
    } finally {
      setScanning(false);
    }
  };

  const runGroup = async (group: MergeGroup) => {
    setApplying(group.canonico);
    setError(null);
    try {
      setNote(await apply(group));
      // Any group sharing these names is stale once they have moved.
      const names = [group.canonico, ...group.variantes];
      setGroups((prev) =>
        prev
          ? prev.filter(
              (other) =>
                !names.includes(other.canonico) &&
                !other.variantes.some((variant) => names.includes(variant)),
            )
          : prev,
      );
    } catch (err) {
      setError(t(keys.failed, { message: String(err) }));
    } finally {
      setApplying(null);
    }
  };

  return (
    <section className="merge-suggest">
      <span className="field-label">{t(keys.title)}</span>
      <span className="muted small">{t(keys.note)}</span>
      <button
        className="ghost-btn"
        onClick={() => void run()}
        disabled={disabled || scanning || applying !== null}
      >
        {scanning ? t(keys.scanning) : t(keys.scan)}
      </button>
      {note && <span className="muted small">{note}</span>}
      {groups?.length === 0 && <span className="muted small">{t(keys.none)}</span>}
      {groups && groups.length > 0 && (
        <ul className="merge-suggest-list">
          {groups.map((group) => (
            <li key={group.canonico} className="merge-suggest-row">
              <span className="merge-suggest-names">
                <strong>{group.canonico}</strong>
                <span className="muted small">
                  {" ← "}
                  {group.variantes.join(" · ")}
                </span>
              </span>
              <button
                className="primary-btn merge-suggest-apply"
                disabled={applying !== null}
                onClick={() => void runGroup(group)}
              >
                {applying === group.canonico ? t(keys.applying) : t(keys.apply)}
              </button>
            </li>
          ))}
        </ul>
      )}
      {error && <span className="err">{error}</span>}
    </section>
  );
}
