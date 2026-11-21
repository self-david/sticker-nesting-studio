import { useEffect, useMemo, useRef, useState } from "react";
import type { Sticker } from "../types";
import type { SheetCopy, SheetSettings } from "../lib/sheet";
import {
  MAX_SIZE_MM,
  MIN_SIZE_MM,
  clamp,
  layoutSheet,
  isQuarterTurn,
  layoutStats,
  maxMarginMm,
  pageDimensions,
} from "../lib/sheet";
import {
  assetUrl,
  exportSheetPdf,
  pickPdfPath,
  stickerContour,
  type PdfExportPage,
} from "../lib/api";
import { useI18n } from "../lib/i18n";
import { fmtMm, round1 } from "../lib/format";
import { Thumb } from "./Thumb";
import { StickerPicker } from "./StickerPicker";
import {
  cachedContours,
  rememberContour,
  rememberFailure,
  retryFailedContours,
  type ContourPolygon,
} from "../lib/contour";

interface Props {
  copies: SheetCopy[];
  settings: SheetSettings;
  onSettings: (patch: Partial<SheetSettings>) => void;
  onAddSticker: (sticker: Sticker, quantity: number) => void;
  onGoLibrary: () => void;
  onUpdateCopy: (id: string, patch: Partial<Pick<SheetCopy, "widthMm" | "heightMm">>) => void;
  onDuplicate: (id: string) => void;
  onRotate: (id: string) => void;
  onSaveProject: () => void;
  onOpenProject: () => void;
  onOpenProjectFolder: () => void;
  onRemove: (id: string) => void;
  onClear: () => void;
  onNotify: (text: string, kind: "ok" | "err") => void;
}

interface MmInputProps {
  value: number;
  min: number;
  max: number;
  onCommit: (value: number) => void;
  ariaLabel: string;
  className?: string;
}

function MmInput({ value, min, max, onCommit, ariaLabel, className }: MmInputProps) {
  const [text, setText] = useState(String(round1(value)));

  useEffect(() => {
    setText(String(round1(value)));
  }, [value]);

  const commit = () => {
    const parsed = Number(text.replace(",", "."));
    if (Number.isFinite(parsed) && parsed >= min && parsed <= max) {
      const next = round1(parsed);
      setText(String(next));
      if (next !== round1(value)) onCommit(next);
    } else {
      setText(String(round1(value)));
    }
  };

  // Arrow keys step one millimetre at a time, inside the same limits typing uses.
  const nudge = (delta: number) => {
    const parsed = Number(text.replace(",", "."));
    const base = Number.isFinite(parsed) ? parsed : value;
    const next = round1(clamp(base + delta, min, max));
    setText(String(next));
    if (next !== round1(value)) onCommit(next);
  };

  return (
    <input
      className={`text-input num mini${className ? ` ${className}` : ""}`}
      value={text}
      inputMode="decimal"
      aria-label={ariaLabel}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          (e.target as HTMLInputElement).blur();
          return;
        }
        if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
        e.preventDefault();
        nudge(e.key === "ArrowUp" ? 1 : -1);
      }}
    />
  );
}

const contourKey = (copy: SheetCopy, pad: number) =>
  `${copy.filePath || copy.thumbPath}|${round1(copy.widthMm)}x${round1(copy.heightMm)}|${pad}`;

/** Where the artwork actually sits inside its (possibly letterboxed) box. */
const artBox = (copy: SheetCopy, widthMm: number, heightMm: number) => {
  const aspect = copy.aspect > 0 ? copy.aspect : widthMm / heightMm;
  const w = widthMm / heightMm > aspect ? heightMm * aspect : widthMm;
  const h = w / aspect;
  return { x: (widthMm - w) / 2, y: (heightMm - h) / 2, w, h };
};

const polygonPath = (points: [number, number][]) =>
  `${points.map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(5)} ${y.toFixed(5)}`).join(" ")} Z`;

export function SheetView({
  copies,
  settings,
  onSettings,
  onAddSticker,
  onGoLibrary,
  onUpdateCopy,
  onDuplicate,
  onRotate,
  onSaveProject,
  onOpenProject,
  onOpenProjectFolder,
  onRemove,
  onClear,
  onNotify,
}: Props) {
  const { t, locale } = useI18n();
  const stageRef = useRef<HTMLDivElement>(null);
  const openMenuRef = useRef<HTMLDivElement>(null);
  const [openMenu, setOpenMenu] = useState(false);
  const [stage, setStage] = useState({ w: 0, h: 0 });
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pageIndex, setPageIndex] = useState(0);
  const [exporting, setExporting] = useState(false);
  const [contours, setContours] = useState<Record<string, ContourPolygon[]>>(() =>
    cachedContours(),
  );

  // The sheet can also be opened from a folder saved with the old format.
  useEffect(() => {
    if (!openMenu) return;
    const close = (event: PointerEvent) => {
      if (!openMenuRef.current?.contains(event.target as Node)) setOpenMenu(false);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [openMenu]);

  // Coming back to the sheet retries whatever failed last time.
  useEffect(() => {
    if (retryFailedContours() > 0) setContours(cachedContours());
  }, []);

  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const update = () => setStage({ w: el.clientWidth, h: el.clientHeight });
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const layout = useMemo(() => layoutSheet(copies, settings), [copies, settings]);
  const stats = layoutStats(layout);
  const pageCount = layout.pages.length;

  useEffect(() => {
    setPageIndex((prev) => Math.min(prev, pageCount - 1));
  }, [pageCount]);

  const currentIndex = Math.min(pageIndex, pageCount - 1);
  const placements = layout.pages[currentIndex] ?? [];
  const scale =
    stage.w > 0 && stage.h > 0
      ? Math.max(0.08, Math.min((stage.w - 56) / layout.pageW, (stage.h - 56) / layout.pageH, 3.5))
      : 1;
  const paperLabel =
    settings.paper === "a4" ? t("sheet.paper.a4") : t("sheet.paper.letter");

  const marginLimit = maxMarginMm(layout.pageW, layout.pageH);

  // Silhouettes are traced in Rust and cached per file, size and padding.
  useEffect(() => {
    if (!settings.cutLines) return;
    const pending = new Map<string, { path: string; widthMm: number; heightMm: number }>();
    for (const page of layout.pages) {
      for (const p of page) {
        const path = p.copy.filePath || p.copy.thumbPath;
        if (!path) continue;
        const key = contourKey(p.copy, settings.outlinePadMm);
        if (!contours[key] && !pending.has(key)) {
          // Padding is measured against the artwork, not the rotated footprint.
          const box = isQuarterTurn(p.rotation)
            ? { w: p.heightMm, h: p.widthMm }
            : { w: p.widthMm, h: p.heightMm };
          const art = artBox(p.copy, box.w, box.h);
          pending.set(key, { path, widthMm: art.w, heightMm: art.h });
        }
      }
    }
    if (pending.size === 0) return;
    let cancelled = false;
    void (async () => {
      const results = await Promise.all(
        [...pending.entries()].map(async ([key, item]) => {
          try {
            const shape = await stickerContour(
              item.path,
              item.widthMm,
              item.heightMm,
              settings.outlinePadMm,
            );
            return [key, shape] as const;
          } catch {
            // A failed trace still shows a cut line, but is retried next visit.
            return [key, null] as const;
          }
        }),
      );
      if (cancelled) return;
      for (const [key, shape] of results) {
        if (shape && shape.length > 0) rememberContour(key, shape);
        else rememberFailure(key);
      }
      setContours(cachedContours());
    })();
    return () => {
      cancelled = true;
    };
  }, [layout, settings.cutLines, settings.outlinePadMm, contours]);

  const exportPages = useMemo<PdfExportPage[]>(() => {
    const pages = layout.pages.filter((placements) => placements.length > 0);
    return pages.map((placements) => ({
      widthMm: layout.pageW,
      heightMm: layout.pageH,
      items: placements.map((p) => ({
        path: p.copy.filePath || p.copy.thumbPath,
        xMm: p.x,
        yMm: p.y,
        widthMm: p.widthMm,
        heightMm: p.heightMm,
        rotation: p.rotation,
      })),
    }));
  }, [layout]);

  const handleExport = async () => {
    if (exporting || exportPages.length === 0) return;
    if (layout.unplaced.length > 0) {
      const proceed = window.confirm(
        t("sheet.exportPartial", { n: layout.unplaced.length, placed: stats.placed }),
      );
      if (!proceed) return;
    }
    try {
      const target = await pickPdfPath("hoja-stickers.pdf");
      if (!target) return;
      setExporting(true);
      const saved = await exportSheetPdf(
        target,
        exportPages,
        settings.cutLines,
        settings.outlineWidthMm,
        settings.outlinePadMm,
      );
      onNotify(t("sheet.exported", { path: saved, n: exportPages.length }), "ok");
    } catch (err) {
      onNotify(String(err), "err");
    } finally {
      setExporting(false);
    }
  };

  return (
    <div className="sheet-view">
      <header className="toolbar">
        <div className="sheet-heading">
          <h1>{t("sheet.title")}</h1>
          <span className="muted small">
            {t("sheet.subtitle", {
              paper: paperLabel,
              w: fmtMm(layout.areaW, locale),
              h: fmtMm(layout.areaH, locale),
            })}
          </span>
        </div>
        <div className="toolbar-right">
          <button className="ghost-btn" onClick={onGoLibrary}>
            {t("sheet.goLibrary")}
          </button>
          <button
            className="ghost-btn"
            onClick={onSaveProject}
            disabled={copies.length === 0}
            title={t("sheet.saveProjectHint")}
          >
            {t("sheet.saveProject")}
          </button>
          <div className="open-group" ref={openMenuRef}>
            <button className="ghost-btn" onClick={onOpenProject} title={t("sheet.openProjectHint")}>
              {t("sheet.openProject")}
            </button>
            <button
              className="ghost-btn open-more"
              onClick={() => setOpenMenu((open) => !open)}
              aria-haspopup="menu"
              aria-expanded={openMenu}
              aria-label={t("sheet.openMore")}
              title={t("sheet.openMore")}
            >
              ▾
            </button>
            {openMenu && (
              <div className="open-menu" role="menu">
                <button
                  role="menuitem"
                  onClick={() => {
                    setOpenMenu(false);
                    onOpenProjectFolder();
                  }}
                >
                  {t("sheet.openLegacyFolder")}
                </button>
              </div>
            )}
          </div>
          <button
            className="ghost-btn"
            onClick={() => void handleExport()}
            disabled={exporting || exportPages.length === 0}
            title={exportPages.length === 0 ? t("sheet.exportEmpty") : undefined}
          >
            {exporting ? t("sheet.exporting") : t("sheet.export")}
          </button>
          <button className="primary-btn" onClick={() => setPickerOpen(true)}>
            + {t("sheet.addStickers")}
          </button>
        </div>
      </header>

      <div className="sheet-body">
        <section className="sheet-stage-col">
          <div className="sheet-stage" ref={stageRef}>
            <div
              className="sheet-page"
              style={{ width: layout.pageW * scale, height: layout.pageH * scale }}
            >
              <div
                className="sheet-area"
                style={{
                  left: layout.areaX * scale,
                  top: layout.areaY * scale,
                  width: layout.areaW * scale,
                  height: layout.areaH * scale,
                }}
              />
              {placements.map((p) => {
                const shape = settings.cutLines
                  ? contours[contourKey(p.copy, settings.outlinePadMm)]
                  : undefined;
                // The copy is laid out rotated; art and outline turn together.
                const box = isQuarterTurn(p.rotation)
                  ? { w: p.heightMm, h: p.widthMm }
                  : { w: p.widthMm, h: p.heightMm };
                const art = artBox(p.copy, box.w, box.h);
                return (
                  <div
                    key={p.copy.id}
                    className="sheet-item"
                    style={{
                      left: p.x * scale,
                      top: p.y * scale,
                      width: p.widthMm * scale,
                      height: p.heightMm * scale,
                    }}
                    title={`${p.copy.name} · ${fmtMm(p.widthMm, locale)} × ${fmtMm(p.heightMm, locale)} mm`}
                  >
                    {/* Art and outline share one box, so a preview never drifts
                        from the cut line drawn around it. */}
                    <div
                      className="sheet-art"
                      style={{
                        width: art.w * scale,
                        height: art.h * scale,
                        transform: `rotate(${p.rotation}deg)`,
                      }}
                    >
                      <Thumb
                        src={assetUrl(p.copy.thumbPath)}
                        ext={p.copy.ext}
                        alt={p.copy.name}
                        compact={Math.min(art.w, art.h) * scale < 64}
                      />
                      {shape && (
                        <svg
                          className="sheet-outline"
                          viewBox="0 0 1 1"
                          preserveAspectRatio="none"
                          aria-hidden="true"
                          style={{
                            inset: 0,
                            width: "100%",
                            height: "100%",
                          }}
                        >
                          {shape.map((polygon, i) => (
                            <path
                              key={i}
                              d={polygonPath(polygon)}
                              vectorEffect="non-scaling-stroke"
                              strokeWidth={Math.max(0.7, settings.outlineWidthMm * scale)}
                            />
                          ))}
                        </svg>
                      )}
                    </div>
                  </div>
                );
              })}
              {copies.length === 0 && <div className="sheet-hint">{t("sheet.hint")}</div>}
            </div>
          </div>

          <div className="sheet-footer">
            <div className="sheet-pager">
              <button
                className="icon-btn"
                onClick={() => setPageIndex((i) => Math.max(0, i - 1))}
                disabled={currentIndex === 0}
                title={t("sheet.prev")}
              >
                ‹
              </button>
              <span className="small">{t("sheet.pageOf", { i: currentIndex + 1, n: pageCount })}</span>
              <button
                className="icon-btn"
                onClick={() => setPageIndex((i) => Math.min(pageCount - 1, i + 1))}
                disabled={currentIndex >= pageCount - 1}
                title={t("sheet.next")}
              >
                ›
              </button>
            </div>
            <span className="muted small">
              {t("sheet.stats", {
                n: stats.placed,
                pct: Math.round(stats.fill * 100),
              })}
            </span>
          </div>
        </section>

        <aside className="sheet-panel">
          <div className="sheet-panel-block">
            <label className="sheet-field">
              <span className="field-label">{t("sheet.paper")}</span>
              <select
                value={settings.paper}
                onChange={(e) => {
                  const paper = e.target.value as SheetSettings["paper"];
                  const next = pageDimensions({ ...settings, paper });
                  onSettings({
                    paper,
                    marginMm: Math.min(settings.marginMm, maxMarginMm(next.w, next.h)),
                  });
                }}
              >
                <option value="a4">{t("sheet.paper.a4")} · 210 × 297 mm</option>
                <option value="letter">{t("sheet.paper.letter")} · 215,9 × 279,4 mm</option>
              </select>
            </label>

            <div className="sheet-field">
              <span className="field-label">{t("sheet.orientation")}</span>
              <div className="seg">
                <button
                  className={settings.orientation === "portrait" ? "is-active" : ""}
                  onClick={() => onSettings({ orientation: "portrait" })}
                >
                  {t("sheet.portrait")}
                </button>
                <button
                  className={settings.orientation === "landscape" ? "is-active" : ""}
                  onClick={() => onSettings({ orientation: "landscape" })}
                >
                  {t("sheet.landscape")}
                </button>
              </div>
            </div>

            <label className="check-row">
              <input
                type="checkbox"
                checked={settings.cutLines}
                onChange={(e) => onSettings({ cutLines: e.target.checked })}
              />
              <span>{t("sheet.cutLines")}</span>
            </label>

            {settings.cutLines && (
              <div className="sheet-two">
                <label className="sheet-field">
                  <span className="field-label">{t("sheet.outlineWidth")}</span>
                  <div className="input-unit">
                    <MmInput
                      value={settings.outlineWidthMm}
                      min={0.1}
                      max={5}
                      ariaLabel={t("sheet.outlineWidth")}
                      onCommit={(v) => onSettings({ outlineWidthMm: v })}
                    />
                    <span className="unit">{t("unit.mm")}</span>
                  </div>
                </label>
                <label className="sheet-field">
                  <span className="field-label">{t("sheet.outlinePadding")}</span>
                  <div className="input-unit">
                    <MmInput
                      value={settings.outlinePadMm}
                      min={0}
                      max={20}
                      ariaLabel={t("sheet.outlinePadding")}
                      onCommit={(v) => onSettings({ outlinePadMm: v })}
                    />
                    <span className="unit">{t("unit.mm")}</span>
                  </div>
                </label>
              </div>
            )}

            <div className="sheet-two">
              <label className="sheet-field">
                <span className="field-label">{t("sheet.margin")}</span>
                <div className="input-unit">
                  <MmInput
                    value={settings.marginMm}
                    min={0}
                    max={marginLimit}
                    ariaLabel={t("sheet.margin")}
                    onCommit={(v) => onSettings({ marginMm: v })}
                  />
                  <span className="unit">{t("unit.mm")}</span>
                </div>
              </label>
              <label className="sheet-field">
                <span className="field-label">{t("sheet.gap")}</span>
                <div className="input-unit">
                  <MmInput
                    value={settings.gapMm}
                    min={0}
                    max={30}
                    ariaLabel={t("sheet.gap")}
                    onCommit={(v) => onSettings({ gapMm: v })}
                  />
                  <span className="unit">{t("unit.mm")}</span>
                </div>
              </label>
            </div>
          </div>

          <div className="sheet-panel-head">
            <span className="field-label">
              {t("sheet.copiesTitle")} · {copies.length}
            </span>
            {copies.length > 0 && (
              <button className="link-btn" onClick={onClear}>
                {t("sheet.clear")}
              </button>
            )}
          </div>

          <div className="copy-list">
            {copies.length === 0 && <p className="muted small copy-empty">{t("sheet.copiesNone")}</p>}
            {copies.map((copy) => (
              <div className="copy-row" key={copy.id}>
                <div className="copy-thumb">
                  <Thumb src={assetUrl(copy.thumbPath)} ext={copy.ext} alt={copy.name} compact />
                </div>
                <div className="copy-main">
                  <span className="copy-name" title={copy.name}>
                    {copy.name}
                  </span>
                  <div className="copy-size">
                    <MmInput
                      value={copy.widthMm}
                      min={MIN_SIZE_MM}
                      max={MAX_SIZE_MM}
                      ariaLabel={`${copy.name} ${t("detail.width")}`}
                      onCommit={(v) => onUpdateCopy(copy.id, { widthMm: v })}
                    />
                    <span className="copy-x">×</span>
                    <MmInput
                      value={copy.heightMm}
                      min={MIN_SIZE_MM}
                      max={MAX_SIZE_MM}
                      ariaLabel={`${copy.name} ${t("detail.height")}`}
                      onCommit={(v) => onUpdateCopy(copy.id, { heightMm: v })}
                    />
                    <span className="unit-inline">{t("unit.mm")}</span>
                  </div>
                </div>
                <div className="copy-actions">
                  <button
                    className="icon-btn"
                    title={t("sheet.rotate")}
                    aria-label={t("sheet.rotate")}
                    onClick={() => onRotate(copy.id)}
                  >
                    ↻
                  </button>
                  <button
                    className="icon-btn"
                    title={t("sheet.duplicate")}
                    onClick={() => onDuplicate(copy.id)}
                  >
                    ⧉
                  </button>
                  <button
                    className="icon-btn"
                    title={t("sheet.remove")}
                    onClick={() => onRemove(copy.id)}
                  >
                    ✕
                  </button>
                </div>
              </div>
            ))}
          </div>

          {layout.unplaced.length > 0 && (
            <div className="sheet-panel-block sheet-unplaced">
              <span className="field-label">{t("sheet.unplaced", { n: layout.unplaced.length })}</span>
              <div className="unplaced-list">
                {layout.unplaced.map((copy) => (
                  <span className="unplaced-chip" key={copy.id} title={copy.name}>
                    {copy.name}
                    <em>{t("sheet.tooBig")}</em>
                  </span>
                ))}
              </div>
            </div>
          )}
        </aside>
      </div>

      <StickerPicker
        open={pickerOpen}
        onClose={() => setPickerOpen(false)}
        onAdd={onAddSticker}
        sheetCount={copies.length}
      />
    </div>
  );
}
