import { useEffect, useMemo, useState } from "react";
import type { Sticker } from "../types";
import { assetUrl, listStickers } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { fmtMm } from "../lib/format";
import { MAX_COPIES, clamp } from "../lib/sheet";
import { Thumb } from "./Thumb";

interface Props {
  open: boolean;
  onClose: () => void;
  onAdd: (sticker: Sticker, quantity: number) => void;
  sheetCount: number;
}

/**
 * Adds stickers without leaving the sheet: the whole library in a searchable
 * list, each row adding the chosen number of copies.
 */
export function StickerPicker({ open, onClose, onAdd, sheetCount }: Props) {
  const { t, locale } = useI18n();
  const [all, setAll] = useState<Sticker[]>([]);
  const [query, setQuery] = useState("");
  const [quantity, setQuantity] = useState(1);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Read the library the first time it opens; keep it for the next opens.
  useEffect(() => {
    if (!open || all.length > 0) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    listStickers("", [], "name")
      .then((list) => {
        if (!cancelled) setAll(list);
      })
      .catch((err) => {
        if (!cancelled) setError(String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, all.length]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return all;
    return all.filter(
      (s) => s.name.toLowerCase().includes(q) || s.tags.some((tag) => tag.includes(q)),
    );
  }, [all, query]);

  if (!open) return null;

  return (
    <div className="picker-backdrop" role="presentation" onPointerDown={onClose}>
      <div
        className="picker"
        role="dialog"
        aria-modal="true"
        aria-label={t("sheet.addStickers")}
        onPointerDown={(e) => e.stopPropagation()}
      >
        <header className="picker-head">
          <span className="field-label">{t("sheet.addStickers")}</span>
          <button className="icon-btn" onClick={onClose} aria-label={t("detail.close")}>
            ✕
          </button>
        </header>

        <div className="picker-tools">
          <div className="search picker-search">
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t("sheet.pickerSearch")}
              spellCheck={false}
              autoFocus
            />
            {query && (
              <button
                className="search-clear"
                onClick={() => setQuery("")}
                aria-label={t("selection.clear")}
              >
                ✕
              </button>
            )}
          </div>
          <label className="picker-qty">
            <span>{t("sheet.quantity")}</span>
            <input
              className="text-input num mini"
              type="number"
              min={1}
              max={MAX_COPIES}
              value={quantity}
              onChange={(e) => setQuantity(clamp(Number(e.target.value) || 1, 1, MAX_COPIES))}
            />
          </label>
        </div>

        {loading ? (
          <div className="picker-message">
            <span className="spinner" />
          </div>
        ) : error ? (
          <p className="picker-message small">{error}</p>
        ) : shown.length === 0 ? (
          <p className="picker-message small muted">
            {all.length === 0 ? t("sheet.pickerLibraryEmpty") : t("sheet.pickerEmpty")}
          </p>
        ) : (
          <div className="picker-list">
            {shown.map((sticker) => (
              <div className="picker-row" key={sticker.id}>
                <div className="picker-thumb">
                  <Thumb
                    src={assetUrl(sticker.thumbPath)}
                    ext={sticker.ext}
                    alt={sticker.name}
                    compact
                  />
                </div>
                <div className="picker-main">
                  <span className="picker-name" title={sticker.name}>
                    {sticker.name}
                  </span>
                  <span className="picker-meta">
                    {fmtMm(sticker.widthMm, locale)} × {fmtMm(sticker.heightMm, locale)}{" "}
                    {t("unit.mm")}
                  </span>
                </div>
                <button className="ghost-btn picker-add" onClick={() => onAdd(sticker, quantity)}>
                  + {t("sheet.pickerAdd")}
                </button>
              </div>
            ))}
          </div>
        )}

        <footer className="picker-foot">
          <span className="muted small">{t("sheet.pickerInSheet", { n: sheetCount })}</span>
          <button className="primary-btn" onClick={onClose}>
            {t("sheet.pickerDone")}
          </button>
        </footer>
      </div>
    </div>
  );
}
