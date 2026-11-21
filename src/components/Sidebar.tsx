import type { LibraryInfo, TagCount } from "../types";
import { useI18n } from "../lib/i18n";
import { fmtBytes } from "../lib/format";

interface Props {
  view: "library" | "sheet";
  onViewChange: (view: "library" | "sheet") => void;
  tags: TagCount[];
  activeTags: string[];
  onToggleTag: (tag: string) => void;
  onClearTags: () => void;
  info: LibraryInfo | null;
  sheetCount: number;
  onOpenFolder: () => void;
  aiPending: number;
  onOpenAi: () => void;
}

export function Sidebar({
  view,
  onViewChange,
  tags,
  activeTags,
  onToggleTag,
  onClearTags,
  info,
  sheetCount,
  onOpenFolder,
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

      <div className="sidebar-section">
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
          </>
        )}
      </div>
    </aside>
  );
}
