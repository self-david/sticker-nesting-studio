import { useEffect, useMemo, useRef, useState } from "react";
import type { Sticker } from "../types";
import type { SheetCopy, SheetSettings } from "../lib/sheet";
import {
  DEFAULT_BORDER_COLOR,
  MAX_AREA_CM2,
  MAX_PAPER_MM,
  MAX_SIZE_MM,
  MIN_PAPER_MM,
  MIN_SIZE_MM,
  clamp,
  maxMarginMm,
  normalizeColor,
  pageDimensions,
  sheetFileName,
} from "../lib/sheet";
import { emptyLayout, layoutStats, nestSheet, type SheetLayout } from "../lib/nest";
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
import { RulerPanel } from "./RulerPanel";
import { pxPerMm as storedPxPerMm, rememberPxPerMm } from "../lib/scale";
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
  /** Gives every copy on the sheet the area set above. */
  onApplyArea: () => void;
  /** Puts back the sizes the copies had before that. */
  onUndoArea: () => void;
  canUndoArea: boolean;
  onSaveProject: () => void;
  onOpenProject: () => void;
  /** Name of the sheet saved inside the app, empty when it has none yet. */
  sheetName: string;
  onSaveLocal: () => void;
  onOpenSheets: () => void;
  /** Asks the user and resolves once one of the buttons is pressed. */
  onConfirm: (message: string) => Promise<boolean>;
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
  onApplyArea,
  onUndoArea,
  canUndoArea,
  onSaveProject,
  onOpenProject,
  sheetName,
  onSaveLocal,
  onOpenSheets,
  onConfirm,
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
  // What is being typed in the page box; empty means "show the real page".
  const [pageDraft, setPageDraft] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [zoom, setZoom] = useState<"fit" | "real">("fit");
  const [rulerOpen, setRulerOpen] = useState(false);
  const [pxPerMm, setPxPerMm] = useState(() => storedPxPerMm());
  // One node per copy and per list row: picking either one brings the other
  // into view, so the two halves of the screen always agree on the selection.
  const itemsRef = useRef(new Map<string, HTMLDivElement>());
  const rowsRef = useRef(new Map<string, HTMLDivElement>());
  // Which half of the screen the last click came from.
  const pickedOnSheet = useRef(false);
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

  // The toast callback is a new function on every render of the app, so it is
  // held in a ref: depending on it would restart the nesting forever.
  const notifyRef = useRef(onNotify);
  notifyRef.current = onNotify;

  // The layout comes from the Rust nester: it traces every silhouette and
  // packs them, which takes a moment on a full sheet.
  const [layout, setLayout] = useState<SheetLayout>(() => emptyLayout(settings));
  const [nesting, setNesting] = useState(false);

  // Debounced, so dragging a size does not start one run per keystroke. A run
  // started before the last edit finishes is ignored when it lands.
  useEffect(() => {
    let cancelled = false;
    setNesting(true);
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const next = await nestSheet(copies, settings);
          if (!cancelled) setLayout(next);
        } catch (err) {
          if (!cancelled) notifyRef.current(String(err), "err");
        } finally {
          if (!cancelled) setNesting(false);
        }
      })();
    }, 300);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [copies, settings]);

  const stats = layoutStats(layout);
  const pageCount = layout.pages.length;

  // Before the first nesting finishes there are no pages at all: the index must
  // not sink to -1 there, or the sheet stays blank once the layout lands.
  useEffect(() => {
    setPageIndex((prev) => Math.max(0, Math.min(prev, pageCount - 1)));
  }, [pageCount]);

  const currentIndex = Math.max(0, Math.min(pageIndex, pageCount - 1));
  const placements = layout.pages[currentIndex] ?? [];

  /** Typed page: go there, or stay put if what was typed is not a page. */
  const commitPage = (draft: string) => {
    setPageDraft("");
    const number = Number.parseInt(draft, 10);
    if (!Number.isFinite(number)) return;
    setPageIndex(Math.max(0, Math.min(pageCount - 1, number - 1)));
  };
  const fitScale =
    stage.w > 0 && stage.h > 0
      ? Math.max(0.08, Math.min((stage.w - 56) / layout.pageW, (stage.h - 56) / layout.pageH, 3.5))
      : 1;
  // Real size comes from the calibration: 1 mm of paper is `pxPerMm` px here.
  const scale = zoom === "real" ? pxPerMm : fitScale;
  const paperLabel =
    settings.paper === "custom"
      ? `${t("sheet.paper.custom")} ${fmtMm(settings.customWidthMm, locale)} × ${fmtMm(settings.customHeightMm, locale)} mm`
      : settings.paper === "a4"
        ? t("sheet.paper.a4")
        : t("sheet.paper.letter");

  // A copy removed on its own must not leave the highlight hanging.
  useEffect(() => {
    if (selected && !copies.some((copy) => copy.id === selected)) setSelected(null);
  }, [copies, selected]);

  /** A row picked in the list brings its copy on the paper into view. */
  const selectCopy = (id: string) => {
    pickedOnSheet.current = false;
    setSelected(id);
    const page = layout.pages.findIndex((placed) => placed.some((p) => p.copy.id === id));
    if (page >= 0 && page !== currentIndex) setPageIndex(page);
  };

  const centreRow = (id: string) =>
    rowsRef.current.get(id)?.scrollIntoView({ block: "center", behavior: "smooth" });

  /** A copy picked on the paper centres its row in the list instead. */
  const pickOnSheet = (id: string) => {
    pickedOnSheet.current = true;
    // Clicking the copy again changes no state, so the effect would not run.
    if (id === selected) centreRow(id);
    setSelected(id);
  };

  // Once the page is the right one and it has been painted, the other half of
  // the screen is scrolled to the copy: the sheet at real size and the list
  // are both longer than the window.
  useEffect(() => {
    if (!selected) return;
    const onSheet = pickedOnSheet.current;
    pickedOnSheet.current = false;
    if (onSheet) {
      centreRow(selected);
      return;
    }
    itemsRef.current.get(selected)?.scrollIntoView({
      block: "center",
      inline: "center",
      behavior: "smooth",
    });
  }, [selected, currentIndex, scale]);

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
          // Padding is measured against the artwork, not the footprint.
          pending.set(key, {
            path,
            widthMm: p.copy.widthMm,
            heightMm: p.copy.heightMm,
          });
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
        rotation: p.rotation,
        artXMm: p.artX,
        artYMm: p.artY,
        artWMm: p.copy.widthMm,
        artHMm: p.copy.heightMm,
      })),
    }));
  }, [layout]);

  const handleExport = async () => {
    if (exporting || exportPages.length === 0) return;
    if (layout.unplaced.length > 0) {
      const proceed = await onConfirm(
        t("sheet.exportPartial", { n: layout.unplaced.length, placed: stats.placed }),
      );
      if (!proceed) return;
    }
    try {
      const target = await pickPdfPath(sheetFileName(sheetName, "hoja-stickers", "pdf"));
      if (!target) return;
      setExporting(true);
      const saved = await exportSheetPdf(
        target,
        exportPages,
        settings.cutLines,
        settings.outlineWidthMm,
        settings.outlinePadMm,
        settings.outlineColor,
        settings.stickerBg,
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
          {sheetName && <span className="sheet-name-chip">{sheetName}</span>}
        </div>
        <div className="toolbar-right">
          <button className="ghost-btn" onClick={onGoLibrary}>
            {t("sheet.goLibrary")}
          </button>
          <button
            className="ghost-btn"
            onClick={onSaveLocal}
            disabled={copies.length === 0}
            title={t("sheet.saveLocalHint")}
          >
            {t("sheet.saveLocal")}
          </button>
          <button className="ghost-btn" onClick={onOpenSheets} title={t("sheet.sheetsTitle")}>
            {t("sheet.mySheets")}
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
          <div className={`sheet-stage${zoom === "real" ? " is-real" : ""}`} ref={stageRef}>
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
                // The artwork turns inside its footprint around its own
                // centre, so the drawn outline travels with it.
                const artW = p.copy.widthMm * scale;
                const artH = p.copy.heightMm * scale;
                return (
                  <div
                    key={p.copy.id}
                    className={`sheet-item${p.copy.id === selected ? " is-selected" : ""}`}
                    ref={(node) => {
                      if (node) itemsRef.current.set(p.copy.id, node);
                      else itemsRef.current.delete(p.copy.id);
                    }}
                    onClick={() => pickOnSheet(p.copy.id)}
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
                        left: p.artX * scale,
                        top: p.artY * scale,
                        width: artW,
                        height: artH,
                        transform: `rotate(${p.rotation}deg)`,
                      }}
                    >
                      {shape && settings.stickerBg !== "" && (
                        <svg
                          className="sheet-outline sheet-sticker-bg"
                          viewBox="0 0 1 1"
                          preserveAspectRatio="none"
                          aria-hidden="true"
                          style={{ inset: 0, width: "100%", height: "100%" }}
                        >
                          {shape.map((polygon, i) => (
                            <path
                              key={i}
                              d={polygonPath(polygon)}
                              style={{ fill: settings.stickerBg, stroke: "none" }}
                            />
                          ))}
                        </svg>
                      )}
                      <Thumb
                        src={assetUrl(p.copy.thumbPath)}
                        ext={p.copy.ext}
                        alt={p.copy.name}
                        compact={Math.min(artW, artH) < 64}
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
                              style={{ stroke: settings.outlineColor }}
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
                onClick={() => {
                  setPageDraft("");
                  setPageIndex((i) => Math.max(0, i - 1));
                }}
                disabled={currentIndex === 0}
                title={t("sheet.prev")}
              >
                ‹
              </button>
              <label className="sheet-page-jump">
                <span className="small">{t("sheet.pageLabel")}</span>
                <input
                  className="page-jump"
                  type="text"
                  inputMode="numeric"
                  value={pageDraft === "" ? String(currentIndex + 1) : pageDraft}
                  aria-label={t("sheet.pageJump", { n: pageCount })}
                  onChange={(event) => setPageDraft(event.target.value.replace(/\D/g, "").slice(0, 4))}
                  onBlur={() => commitPage(pageDraft)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      commitPage(pageDraft);
                      event.currentTarget.blur();
                    }
                    if (event.key === "Escape") {
                      setPageDraft("");
                      event.currentTarget.blur();
                    }
                  }}
                />
                <span className="small">{t("sheet.pageOfN", { n: pageCount })}</span>
              </label>
              <button
                className="icon-btn"
                onClick={() => {
                  setPageDraft("");
                  setPageIndex((i) => Math.min(pageCount - 1, i + 1));
                }}
                disabled={currentIndex >= pageCount - 1}
                title={t("sheet.next")}
              >
                ›
              </button>
            </div>
            <span className="muted small">
              {nesting
                ? t("sheet.nesting")
                : t("sheet.stats", {
                    n: stats.placed,
                    ink: Math.round(stats.ink * 100),
                    box: Math.round(stats.box * 100),
                  })}
            </span>
            <div className="sheet-footer-right">
              <div className="seg">
                <button
                  className={zoom === "fit" ? "is-active" : ""}
                  onClick={() => setZoom("fit")}
                  title={t("sheet.zoomFitHint")}
                >
                  {t("sheet.zoomFit")}
                </button>
                <button
                  className={zoom === "real" ? "is-active" : ""}
                  onClick={() => setZoom("real")}
                  title={t("sheet.zoomRealHint")}
                >
                  {t("sheet.zoomReal")}
                </button>
              </div>
              <button className="ghost-btn small-btn" onClick={() => setRulerOpen(true)}>
                {t("sheet.ruler")}
              </button>
            </div>
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
                <option value="custom">{t("sheet.paper.custom")}</option>
              </select>
            </label>

            {settings.paper === "custom" && (
              <div className="sheet-two">
                <label className="sheet-field">
                  <span className="field-label">{t("sheet.customW")}</span>
                  <div className="input-unit">
                    <MmInput
                      value={settings.customWidthMm}
                      min={MIN_PAPER_MM}
                      max={MAX_PAPER_MM}
                      ariaLabel={t("sheet.customW")}
                      onCommit={(v) => {
                        const next = pageDimensions({ ...settings, customWidthMm: v });
                        onSettings({
                          customWidthMm: v,
                          marginMm: Math.min(settings.marginMm, maxMarginMm(next.w, next.h)),
                        });
                      }}
                    />
                    <span className="unit">{t("unit.mm")}</span>
                  </div>
                </label>
                <label className="sheet-field">
                  <span className="field-label">{t("sheet.customH")}</span>
                  <div className="input-unit">
                    <MmInput
                      value={settings.customHeightMm}
                      min={MIN_PAPER_MM}
                      max={MAX_PAPER_MM}
                      ariaLabel={t("sheet.customH")}
                      onCommit={(v) => {
                        const next = pageDimensions({ ...settings, customHeightMm: v });
                        onSettings({
                          customHeightMm: v,
                          marginMm: Math.min(settings.marginMm, maxMarginMm(next.w, next.h)),
                        });
                      }}
                    />
                    <span className="unit">{t("unit.mm")}</span>
                  </div>
                </label>
              </div>
            )}

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

            {settings.cutLines && (
              <div className="sheet-two">
                <label className="sheet-field">
                  <span className="field-label">{t("sheet.outlineColor")}</span>
                  <input
                    className="color-input"
                    type="color"
                    value={settings.outlineColor}
                    aria-label={t("sheet.outlineColor")}
                    onChange={(event) =>
                      onSettings({
                        outlineColor: normalizeColor(event.target.value, DEFAULT_BORDER_COLOR),
                      })
                    }
                  />
                </label>
                <label className="sheet-field">
                  <span className="field-label">{t("sheet.stickerBg")}</span>
                  <span className="color-row">
                    <input
                      type="checkbox"
                      checked={settings.stickerBg !== ""}
                      aria-label={t("sheet.stickerBgOn")}
                      onChange={(event) =>
                        onSettings({ stickerBg: event.target.checked ? "#ffffff" : "" })
                      }
                    />
                    <input
                      className="color-input"
                      type="color"
                      value={settings.stickerBg || "#ffffff"}
                      disabled={settings.stickerBg === ""}
                      aria-label={t("sheet.stickerBg")}
                      onChange={(event) =>
                        onSettings({ stickerBg: normalizeColor(event.target.value, "#ffffff") })
                      }
                    />
                  </span>
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

            <div className="sheet-field">
              <span className="field-label">{t("sheet.areaCm2")}</span>
              <div className="input-unit">
                <input
                  className="text-input num mini"
                  type="number"
                  min={0}
                  max={MAX_AREA_CM2}
                  step={1}
                  value={settings.areaCm2 || ""}
                  placeholder="—"
                  aria-label={t("sheet.areaCm2")}
                  onChange={(event) =>
                    onSettings({
                      areaCm2: clamp(Number(event.target.value) || 0, 0, MAX_AREA_CM2),
                    })
                  }
                />
                <span className="unit">{t("unit.cm2")}</span>
              </div>
              <span className="muted small">{t("sheet.areaCm2Hint")}</span>
              <div className="area-actions">
                <button
                  className="ghost-btn small-btn"
                  onClick={onApplyArea}
                  disabled={!settings.areaCm2 || copies.length === 0}
                  title={t("sheet.areaApplyHint")}
                >
                  {t("sheet.areaApply", { n: copies.length })}
                </button>
                <button
                  className="link-btn"
                  onClick={onUndoArea}
                  disabled={!canUndoArea}
                  title={t("sheet.areaUndoHint")}
                >
                  {t("sheet.areaUndo")}
                </button>
              </div>
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
              <div
                className={`copy-row${copy.id === selected ? " is-selected" : ""}`}
                key={copy.id}
                ref={(node) => {
                  if (node) rowsRef.current.set(copy.id, node);
                  else rowsRef.current.delete(copy.id);
                }}
                onClick={() => selectCopy(copy.id)}
              >
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

      {rulerOpen && (
        <RulerPanel
          value={pxPerMm}
          onClose={() => setRulerOpen(false)}
          onSave={(value) => {
            rememberPxPerMm(value);
            setPxPerMm(storedPxPerMm());
            setRulerOpen(false);
          }}
        />
      )}
    </div>
  );
}
