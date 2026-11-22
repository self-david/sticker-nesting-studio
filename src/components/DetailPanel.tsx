import { useEffect, useState } from "react";
import type { FacetCount, Sticker } from "../types";
import { useI18n } from "../lib/i18n";
import { TIPOS } from "../lib/tipos";
import { assetUrl } from "../lib/api";
import { fmtBytes, fmtDate, fmtMm, round1 } from "../lib/format";
import { Thumb } from "./Thumb";

interface Props {
  sticker: Sticker;
  selectionCount: number;
  onClose: () => void;
  onPatch: (id: string, patch: { name?: string; widthMm?: number; heightMm?: number }) => Promise<void>;
  onTags: (id: string, tags: string[]) => Promise<void>;
  /** Series already in use, for the autocomplete. */
  series: FacetCount[];
  onClassification: (id: string, serie: string, tipo: string) => Promise<void>;
  onDelete: (id: string) => void;
  onAi: (id: string) => void;
  aiBusy: boolean;
}

export function DetailPanel({
  sticker,
  selectionCount,
  onClose,
  onPatch,
  onTags,
  series,
  onClassification,
  onDelete,
  onAi,
  aiBusy,
}: Props) {
  const { t, locale } = useI18n();
  const [name, setName] = useState(sticker.name);
  const [width, setWidth] = useState(String(round1(sticker.widthMm)));
  const [height, setHeight] = useState(String(round1(sticker.heightMm)));
  const [lock, setLock] = useState(true);
  const [tagInput, setTagInput] = useState("");
  const [serieInput, setSerieInput] = useState(sticker.serie);
  const [busy, setBusy] = useState(false);

  const ratio = sticker.widthMm / sticker.heightMm || 1;

  useEffect(() => {
    setName(sticker.name);
    setWidth(String(round1(sticker.widthMm)));
    setHeight(String(round1(sticker.heightMm)));
    setTagInput("");
    setSerieInput(sticker.serie);
  }, [sticker.id, sticker.name, sticker.widthMm, sticker.heightMm, sticker.serie]);

  const commitName = async () => {
    const trimmed = name.trim();
    if (!trimmed || trimmed === sticker.name) {
      setName(sticker.name);
      return;
    }
    setBusy(true);
    try {
      await onPatch(sticker.id, { name: trimmed });
    } finally {
      setBusy(false);
    }
  };

  const commitSerie = async () => {
    const trimmed = serieInput.trim();
    if (trimmed === sticker.serie) {
      setSerieInput(sticker.serie);
      return;
    }
    setBusy(true);
    try {
      await onClassification(sticker.id, trimmed, sticker.tipo);
    } finally {
      setBusy(false);
    }
  };

  const commitSize = async (which: "w" | "h", rawValue?: string) => {
    const raw = rawValue ?? (which === "w" ? width : height);
    const value = Number(raw.replace(",", "."));
    if (!Number.isFinite(value) || value < 1 || value > 2000) {
      setWidth(String(round1(sticker.widthMm)));
      setHeight(String(round1(sticker.heightMm)));
      return;
    }
    let w = which === "w" ? round1(value) : sticker.widthMm;
    let h = which === "h" ? round1(value) : sticker.heightMm;
    if (lock) {
      if (which === "w") h = round1(w / ratio);
      else w = round1(h * ratio);
    }
    setWidth(String(w));
    setHeight(String(h));
    setBusy(true);
    try {
      await onPatch(sticker.id, { widthMm: w, heightMm: h });
    } finally {
      setBusy(false);
    }
  };

  // Arrow keys change the size by one millimetre; the aspect lock still wins.
  const nudgeSize = (which: "w" | "h", delta: number) => {
    const typed = Number((which === "w" ? width : height).replace(",", "."));
    const current = which === "w" ? sticker.widthMm : sticker.heightMm;
    const base = Number.isFinite(typed) && typed >= 1 && typed <= 2000 ? typed : current;
    const next = round1(base + delta);
    if (next < 1 || next > 2000 || next === round1(base)) return;
    void commitSize(which, String(next));
  };

  const addTag = async () => {
    const tag = tagInput.trim();
    if (!tag) return;
    setTagInput("");
    if (sticker.tags.includes(tag.toLowerCase())) return;
    await onTags(sticker.id, [...sticker.tags, tag]);
  };

  const removeTag = async (tag: string) => {
    await onTags(
      sticker.id,
      sticker.tags.filter((x) => x !== tag),
    );
  };

  return (
    <aside className="detail">
      <header className="detail-head">
        <span className="detail-kicker">Sticker</span>
        <button className="icon-btn" onClick={onClose} title={t("detail.close")}>
          ✕
        </button>
      </header>

      <div className="detail-preview">
        <Thumb src={assetUrl(sticker.thumbPath)} ext={sticker.ext} alt={sticker.name} />
      </div>

      <label className="field">
        <span className="field-label">{t("detail.name")}</span>
        <input
          className="text-input"
          value={name}
          disabled={busy}
          onChange={(e) => setName(e.target.value)}
          onBlur={commitName}
          onKeyDown={(e) => {
            if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          }}
        />
      </label>

      <div className="field-grid">
        <label className="field">
          <span className="field-label">{t("detail.serie")}</span>
          <input
            className="text-input"
            list="detail-series"
            value={serieInput}
            placeholder={t("detail.seriePlaceholder")}
            disabled={busy}
            onChange={(e) => setSerieInput(e.target.value)}
            onBlur={commitSerie}
            onKeyDown={(e) => {
              if (e.key === "Enter") (e.target as HTMLInputElement).blur();
            }}
          />
        </label>
        <label className="field">
          <span className="field-label">{t("detail.tipo")}</span>
          <select
            className="text-input detail-tipo"
            value={sticker.tipo}
            disabled={busy}
            onChange={(e) => void onClassification(sticker.id, sticker.serie, e.target.value)}
          >
            <option value="">{t("facet.noTipo")}</option>
            {TIPOS.map((tipo) => (
              <option key={tipo} value={tipo}>
                {tipo}
              </option>
            ))}
          </select>
        </label>
      </div>
      <datalist id="detail-series">
        {series
          .filter((facet) => facet.value !== "")
          .map((facet) => (
            <option key={facet.value} value={facet.value} />
          ))}
      </datalist>

      <div className="field">
        <div className="field-row">
          <span className="field-label">{t("detail.size")}</span>
          <button
            className={`toggle${lock ? " is-on" : ""}`}
            onClick={() => setLock((v) => !v)}
            title={t("detail.lockAspect")}
          >
            {lock ? "🔒" : "🔓"} {t("detail.lockAspect")}
          </button>
        </div>
        <div className="size-inputs">
          <label className="size-box">
            <span>{t("detail.width")}</span>
            <div>
              <input
                className="text-input num"
                value={width}
                inputMode="decimal"
                disabled={busy}
                onChange={(e) => setWidth(e.target.value)}
                onBlur={() => commitSize("w")}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    (e.target as HTMLInputElement).blur();
                    return;
                  }
                  if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
                  e.preventDefault();
                  nudgeSize("w", e.key === "ArrowUp" ? 1 : -1);
                }}
              />
              <span className="unit">{t("unit.mm")}</span>
            </div>
          </label>
          <label className="size-box">
            <span>{t("detail.height")}</span>
            <div>
              <input
                className="text-input num"
                value={height}
                inputMode="decimal"
                disabled={busy}
                onChange={(e) => setHeight(e.target.value)}
                onBlur={() => commitSize("h")}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    (e.target as HTMLInputElement).blur();
                    return;
                  }
                  if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
                  e.preventDefault();
                  nudgeSize("h", e.key === "ArrowUp" ? 1 : -1);
                }}
              />
              <span className="unit">{t("unit.mm")}</span>
            </div>
          </label>
        </div>
      </div>

      <div className="field">
        <span className="field-label">{t("detail.tags")}</span>
        <div className="tag-editor">
          {sticker.tags.map((tag) => (
            <span key={tag} className="tag-pill">
              {tag}
              <button onClick={() => removeTag(tag)} aria-label={`quitar ${tag}`}>
                ✕
              </button>
            </span>
          ))}
          <input
            className="tag-input"
            value={tagInput}
            placeholder={t("detail.tagsPlaceholder")}
            onChange={(e) => setTagInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void addTag();
              }
            }}
          />
        </div>
      </div>

      <div className="detail-meta">
        <div className="meta-row">
          <span>{t("detail.file")}</span>
          <strong>{sticker.ext.toUpperCase()}</strong>
        </div>
        <div className="meta-row">
          <span>{t("detail.resolution")}</span>
          <strong>
            {sticker.kind === "vector"
              ? t("kind.vector")
              : `${sticker.widthPx} × ${sticker.heightPx} px · ${sticker.dpi} ppp`}
          </strong>
        </div>
        {sticker.kind === "raster" && (
          <div className="meta-row">
            <span>{t("detail.alpha")}</span>
            <strong>{sticker.hasAlpha ? t("common.yes") : t("common.no")}</strong>
          </div>
        )}
        <div className="meta-row">
          <span>{t("detail.created")}</span>
          <strong>{fmtDate(sticker.createdAt, locale)}</strong>
        </div>
        <div className="meta-row">
          <span>{t("detail.weight")}</span>
          <strong>{fmtBytes(sticker.sizeBytes)}</strong>
        </div>
        <div className="meta-row">
          <span>{t("detail.area")}</span>
          <strong>
            {fmtMm(sticker.widthMm * sticker.heightMm, locale)} mm²
          </strong>
        </div>
      </div>

      <button className="ghost-btn wide-btn" onClick={() => onAi(sticker.id)} disabled={aiBusy}>
        ✦ {aiBusy ? t("detail.aiBusy") : t("detail.ai")}
      </button>

      <button className="danger-btn" onClick={() => onDelete(sticker.id)}>
        {t("detail.delete")}
        {selectionCount > 1 ? ` (${selectionCount})` : ""}
      </button>
    </aside>
  );
}
