import type { FacetCount, LibraryInfo, TagCount } from "../types";
import { useI18n } from "../lib/i18n";
import { fmtBytes } from "../lib/format";

interface Props {
  view: "library" | "sheet";
  onViewChange: (view: "library" | "sheet") => void;
  tags: TagCount[];
  activeTags: string[];
  onToggleTag: (tag: string) => void;
  onClearTags: () => void;
  series: FacetCount[];
  tipos: FacetCount[];
  activeSerie: string | null;
  activeTipo: string | null;
  onSelectSerie: (serie: string | null) => void;
  onSelectTipo: (tipo: string | null) => void;
  onReviewSeries: () => void;
  info: LibraryInfo | null;
  sheetCount: number;
  onOpenFolder: () => void;
  onRecalcDpi: () => void;
  /** A density pass over the library is running. */
  recalcBusy: boolean;
  /** Recompresses the stored artwork and converts any SVG left. */
  onOptimize: () => void;
  optimizeBusy: boolean;
  onExportLibrary: () => void;
  onImportLibrary: () => void;
  /** A whole-library export or import is running. */
  archiveBusy: boolean;
  aiPending: number;
  onOpenAi: () => void;
}

interface FacetListProps {
  items: FacetCount[];
  active: string | null;
  onSelect: (value: string | null) => void;
  /** Label for the empty value: «Sin serie», «Sin tipo». */
  noValue: string;
  empty: string;
  clear: string;
}

/** A single-choice facet: clicking the active value again clears it. */
function FacetList({ items, active, onSelect, noValue, empty, clear }: FacetListProps) {
  return (
    <div className="tag-list">
      {items.length === 0 && <p className="muted small">{empty}</p>}
      {items.map((item) => (
        <button
          key={item.value || "none"}
          className={`tag-chip is-facet${active === item.value ? " is-active" : ""}`}
          onClick={() => onSelect(active === item.value ? null : item.value)}
          title={active === item.value ? clear : undefined}
        >
          <span className="tag-name">{item.value || noValue}</span>
          <span className="tag-count">{item.count}</span>
        </button>
      ))}
    </div>
  );
}

export function Sidebar({
  view,
  onViewChange,
  tags,
  activeTags,
  onToggleTag,
  onClearTags,
  series,
  tipos,
  activeSerie,
  activeTipo,
  onSelectSerie,
  onSelectTipo,
  onReviewSeries,
  info,
  sheetCount,
  onOpenFolder,
  onRecalcDpi,
  recalcBusy,
  onOptimize,
  optimizeBusy,
  onExportLibrary,
  onImportLibrary,
  archiveBusy,
  aiPending,
  onOpenAi,
}: Props) {
  const { t, lang, setLang } = useI18n();

  return (
    <aside className="sidebar">
      <div className="brand">
        <div className="brand-mark" aria-hidden="true">
          <svg viewBox="0 0 32 32" width="28" height="28">
            <rect x="2.5" y="2.5" width="16" height="16" rx="4.5" fill="none" stroke="currentColor" strokeWidth="2" />
            <rect x="13.5" y="13.5" width="16" height="16" rx="4.5" fill="none" stroke="currentColor" strokeWidth="2" opacity="0.55" />
          </svg>
        </div>
        <div className="brand-text">
          <strong>{t("app.name")}</strong>
          <span>{t("app.tagline")}</span>
        </div>
      </div>

      <nav className="nav">
        <button
          className={`nav-item${view === "library" ? " is-active" : ""}`}
          onClick={() => onViewChange("library")}
        >
          <span className="nav-dot" />
          {t("nav.library")}
          {info && <span className="nav-count">{info.count}</span>}
        </button>
        <button
          className={`nav-item${view === "sheet" ? " is-active" : ""}`}
          onClick={() => onViewChange("sheet")}
        >
          <span className="nav-dot" />
          {t("nav.sheet")}
          {sheetCount > 0 && <span className="nav-pill">{sheetCount}</span>}
        </button>
        <button className="nav-item" onClick={onOpenAi}>
          <span className="nav-dot" />
          {t("nav.ai")}
          {aiPending > 0 && <span className="nav-count">{aiPending}</span>}
        </button>
      </nav>

      <div className="sidebar-facets">
        <section className="sidebar-section">
          <div className="sidebar-heading">
            <span>{t("facet.series")}</span>
            <span className="heading-links">
              {activeSerie !== null && (
                <button className="link-btn" onClick={() => onSelectSerie(null)}>
                  {t("facet.clear")}
                </button>
              )}
              <button className="link-btn" onClick={onReviewSeries}>
                {t("series.open")}
              </button>
            </span>
          </div>
          <FacetList
            items={series}
            active={activeSerie}
            onSelect={onSelectSerie}
            noValue={t("facet.noSerie")}
            empty={t("facet.none")}
            clear={t("facet.clear")}
          />
        </section>

        <section className="sidebar-section">
          <div className="sidebar-heading">
            <span>{t("facet.tipos")}</span>
            {activeTipo !== null && (
              <button className="link-btn" onClick={() => onSelectTipo(null)}>
                {t("facet.clear")}
              </button>
            )}
          </div>
          <FacetList
            items={tipos}
            active={activeTipo}
            onSelect={onSelectTipo}
            noValue={t("facet.noTipo")}
            empty={t("facet.none")}
            clear={t("facet.clear")}
          />
        </section>

        <section className="sidebar-section">
          <div className="sidebar-heading">
            <span>{t("tags.title")}</span>
            {activeTags.length > 0 && (
              <button className="link-btn" onClick={onClearTags}>
                {t("tags.clear")}
              </button>
            )}
          </div>
          <div className="tag-list">
            {tags.length === 0 && <p className="muted small">{t("tags.none")}</p>}
            {tags.map((tag) => (
              <button
                key={tag.tag}
                className={`tag-chip${activeTags.includes(tag.tag) ? " is-active" : ""}`}
                onClick={() => onToggleTag(tag.tag)}
              >
                <span className="tag-name">{tag.tag}</span>
                <span className="tag-count">{tag.count}</span>
              </button>
            ))}
          </div>
        </section>
      </div>

      <div className="sidebar-footer">
        <div className="lang-switch" role="group">
          <button className={lang === "es" ? "is-active" : ""} onClick={() => setLang("es")}>
            ES
          </button>
          <button className={lang === "en" ? "is-active" : ""} onClick={() => setLang("en")}>
            EN
          </button>
        </div>
        {info && (
          <>
            <div className="storage-line">
              <span>{t("library.size", { size: fmtBytes(info.bytes) })}</span>
            </div>
            <button className="ghost-btn" onClick={onOpenFolder}>
              {t("library.openFolder")}
            </button>
            <button
              className="ghost-btn"
              onClick={onRecalcDpi}
              disabled={recalcBusy || info.count === 0}
              title={t("library.recalcTitle")}
            >
              {recalcBusy ? t("library.recalcBusy") : t("library.recalc")}
            </button>
            <button
              className="ghost-btn"
              onClick={onOptimize}
              disabled={optimizeBusy || info.count === 0}
              title={t("library.optimizeTitle")}
            >
              {optimizeBusy ? t("library.optimizeBusy") : t("library.optimize")}
            </button>
            <button
              className="ghost-btn"
              onClick={onExportLibrary}
              disabled={archiveBusy || info.count === 0}
            >
              {archiveBusy ? t("library.archiving") : t("library.export")}
            </button>
            <button className="ghost-btn" onClick={onImportLibrary} disabled={archiveBusy}>
              {t("library.import")}
            </button>
          </>
        )}
      </div>
    </aside>
  );
}
