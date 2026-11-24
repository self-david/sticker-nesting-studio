import { useEffect, useMemo, useState } from "react";
import type { FacetCount } from "../types";
import { useI18n } from "../lib/i18n";
import { fold } from "../lib/search";
import { aiSeriesSuggestions } from "../lib/api";
import { MergeSuggestions } from "./MergeSuggestions";

interface Props {
  series: FacetCount[];
  /** A merge is in flight: no closing and no second click. */
  busy: boolean;
  onClose: () => void;
  onMerge: (from: string[], into: string) => Promise<void> | void;
}

/**
 * The cheap safety net against the same series spelled three ways: pick the
 * ones that are the same, type the name that stays and move them in one go.
 */
export function SeriesPanel({ series, busy, onClose, onMerge }: Props) {
  const { t } = useI18n();
  const [picked, setPicked] = useState<string[]>([]);
  const [target, setTarget] = useState("");
  const [needle, setNeedle] = useState("");

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  const named = useMemo(() => series.filter((facet) => facet.value !== ""), [series]);
  const shown = useMemo(() => {
    const wanted = fold(needle.trim());
    if (!wanted) return named;
    return named.filter((facet) => fold(facet.value).includes(wanted));
  }, [named, needle]);

  const toggle = (value: string) =>
    setPicked((prev) =>
      prev.includes(value) ? prev.filter((name) => name !== value) : [...prev, value],
    );

  const ready = picked.length > 0 && target.trim().length > 0 && !busy;

  return (
    <div className="picker-backdrop" onPointerDown={() => !busy && onClose()}>
      <div
        className="picker series-panel"
        role="dialog"
        aria-modal="true"
        aria-label={t("series.title")}
        onPointerDown={(event) => event.stopPropagation()}
      >
        <header className="picker-head">
          <div className="series-headtext">
            <span className="field-label">{t("series.title")}</span>
            <p className="muted small series-note">{t("series.note")}</p>
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

        <div className="search picker-search">
          <input
            value={needle}
            onChange={(event) => setNeedle(event.target.value)}
            placeholder={t("series.search")}
            spellCheck={false}
            autoFocus
          />
          {needle && (
            <button
              className="search-clear"
              onClick={() => setNeedle("")}
              aria-label={t("selection.clear")}
            >
              ✕
            </button>
          )}
        </div>

        {named.length === 0 ? (
          <p className="picker-message small muted">{t("series.none")}</p>
        ) : shown.length === 0 ? (
          <p className="picker-message small muted">{t("series.empty")}</p>
        ) : (
          <div className="picker-list series-list">
            {shown.map((facet) => (
              <label
                key={facet.value}
                className={`series-row${picked.includes(facet.value) ? " is-picked" : ""}`}
              >
                <input
                  type="checkbox"
                  checked={picked.includes(facet.value)}
                  onChange={() => toggle(facet.value)}
                />
                <span className="series-name" title={facet.value}>
                  {facet.value}
                </span>
                <span className="tag-count">{facet.count}</span>
              </label>
            ))}
          </div>
        )}

        <MergeSuggestions
          disabled={busy}
          scan={aiSeriesSuggestions}
          apply={async (group) => {
            await onMerge(group.variantes, group.canonico);
            return null;
          }}
          keys={{
            title: "series.aiTitle",
            note: "series.aiNote",
            scan: "series.aiScan",
            scanning: "series.aiScanning",
            none: "series.aiNone",
            apply: "series.aiApply",
            applying: "series.aiApplying",
            failed: "series.aiFailed",
          }}
        />

        <div className="series-merge">
          <label className="series-target">
            <span className="field-label">{t("series.target")}</span>
            <input
              className="text-input"
              list="series-merge-target"
              value={target}
              placeholder={t("series.targetPlaceholder")}
              disabled={busy}
              onChange={(event) => setTarget(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && ready) onMerge(picked, target);
              }}
            />
          </label>
          <datalist id="series-merge-target">
            {named.map((facet) => (
              <option key={facet.value} value={facet.value} />
            ))}
          </datalist>
          <button
            className="primary-btn"
            disabled={!ready}
            onClick={() => onMerge(picked, target)}
          >
            {target.trim()
              ? t("series.confirm", { name: target.trim() })
              : t("series.confirmPlain")}
          </button>
        </div>

        <footer className="picker-foot">
          <span className="muted small">
            {picked.length > 0
              ? t("series.picked", { n: picked.length })
              : t("series.nonePicked")}
            {picked.length > 0 && (
              <button className="link-btn series-clear" onClick={() => setPicked([])}>
                {t("selection.clear")}
              </button>
            )}
          </span>
          <button className="ghost-btn" onClick={onClose} disabled={busy}>
            {t("common.done")}
          </button>
        </footer>
      </div>
    </div>
  );
}
