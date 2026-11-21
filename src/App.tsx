import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { LibraryInfo, SortKey, Sticker, TagCount } from "./types";
import {
  PROJECT_EXTENSION,
  aiDescribe,
  deleteStickers,
  importPaths,
  isTauri,
  libraryInfo,
  listStickers,
  listTags,
  openSheetProject,
  pickImages,
  pickProjectFile,
  pickProjectFolder,
  pickProjectToOpen,
  revealLibrary,
  saveSheetProject,
  setStickerTags,
  updateSticker,
} from "./lib/api";
import { useI18n } from "./lib/i18n";
import { forgetLastSheet, lastSheet, rememberLastSheet } from "./lib/recent";
import {
  DEFAULT_SETTINGS,
  copiesFromSticker,
  newId,
  resizeCopy,
  rotateCopy,
  type SheetCopy,
  type SheetSettings,
} from "./lib/sheet";
import { Sidebar } from "./components/Sidebar";
import { StickerGrid } from "./components/StickerGrid";
import { DetailPanel } from "./components/DetailPanel";
import { SheetView } from "./components/SheetView";
import { AiPanel } from "./components/AiPanel";

interface Toast {
  kind: "ok" | "err";
  text: string;
}

export default function App() {
  const { t } = useI18n();
  const [view, setView] = useState<"library" | "sheet">("library");
  const [stickers, setStickers] = useState<Sticker[]>([]);
  const [tags, setTags] = useState<TagCount[]>([]);
  const [info, setInfo] = useState<LibraryInfo | null>(null);
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [sort, setSort] = useState<SortKey>("recent");
  const [activeTags, setActiveTags] = useState<string[]>([]);
  const [selection, setSelection] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [importing, setImporting] = useState(false);
  const [dropping, setDropping] = useState(false);
  const [toast, setToast] = useState<Toast | null>(null);
  const [sheetCopies, setSheetCopies] = useState<SheetCopy[]>([]);
  const [sheetSettings, setSheetSettings] = useState<SheetSettings>(DEFAULT_SETTINGS);
  const [aiOpen, setAiOpen] = useState(false);
  const [aiPending, setAiPending] = useState<Sticker[]>([]);
  const [aiBusyId, setAiBusyId] = useState<string | null>(null);
  const anchorRef = useRef<string | null>(null);

  useEffect(() => {
    const id = window.setTimeout(() => setDebounced(query), 180);
    return () => window.clearTimeout(id);
  }, [query]);

  const reload = useCallback(async () => {
    if (!isTauri) {
      setLoading(false);
      return;
    }
    try {
      const [list, tagList, lib] = await Promise.all([
        listStickers(debounced, activeTags, sort),
        listTags(),
        libraryInfo(),
      ]);
      setStickers(list);
      setTags(tagList);
      setInfo(lib);
    } catch (err) {
      setToast({ kind: "err", text: String(err) });
    } finally {
      setLoading(false);
    }
  }, [debounced, activeTags, sort]);

  useEffect(() => {
    void reload();
  }, [reload]);

  // Pending AI work is counted over the whole library, not the filtered view.
  const refreshPending = useCallback(async () => {
    if (!isTauri) return;
    try {
      const all = await listStickers("", [], "recent");
      setAiPending(all.filter((s) => !s.aiAt));
    } catch {
      // The library is not ready yet; the badge just stays empty.
    }
  }, []);

  useEffect(() => {
    void refreshPending();
  }, [refreshPending]);

  useEffect(() => {
    if (!toast) return;
    const id = window.setTimeout(() => setToast(null), 4200);
    return () => window.clearTimeout(id);
  }, [toast]);

  const runImport = useCallback(
    async (paths: string[]) => {
      if (!paths.length || !isTauri) return;
      setImporting(true);
      try {
        const report = await importPaths(paths);
        await reload();
        void refreshPending();
        if (report.importedIds.length > 0) {
          setToast({ kind: "ok", text: t("import.imported", { n: report.importedIds.length }) });
        }
        if (report.errors.length > 0) {
          setToast({ kind: "err", text: t("import.someFailed", { n: report.errors.length }) });
          console.warn("Import errors", report.errors);
        }
      } catch (err) {
        setToast({ kind: "err", text: String(err) });
      } finally {
        setImporting(false);
      }
    },
    [reload, refreshPending, t],
  );

  const handleImportClick = useCallback(async () => {
    try {
      const paths = await pickImages();
      await runImport(paths);
    } catch (err) {
      setToast({ kind: "err", text: String(err) });
    }
  }, [runImport]);

  // Native drag & drop from Windows Explorer.
  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    (async () => {
      try {
        const { getCurrentWebview } = await import("@tauri-apps/api/webview");
        const stop = await getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type === "over") setDropping(true);
          else if (event.payload.type === "drop") {
            setDropping(false);
            void runImport(event.payload.paths);
          } else setDropping(false);
        });
        if (disposed) stop();
        else unlisten = stop;
      } catch (err) {
        console.warn("Drag & drop no disponible", err);
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [runImport]);

  const handleSelect = useCallback(
    (id: string, event: React.MouseEvent) => {
      const additive = event.ctrlKey || event.metaKey;
      const range = event.shiftKey;
      setSelection((prev) => {
        if (additive) {
          anchorRef.current = id;
          return prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id];
        }
        if (range && anchorRef.current) {
          const a = stickers.findIndex((s) => s.id === anchorRef.current);
          const b = stickers.findIndex((s) => s.id === id);
          if (a >= 0 && b >= 0) {
            const [from, to] = a < b ? [a, b] : [b, a];
            return stickers.slice(from, to + 1).map((s) => s.id);
          }
        }
        anchorRef.current = id;
        return [id];
      });
    },
    [stickers],
  );

  const patchSticker = useCallback(
    async (id: string, patch: { name?: string; widthMm?: number; heightMm?: number }) => {
      try {
        await updateSticker(id, patch);
        setStickers((prev) => prev.map((s) => (s.id === id ? { ...s, ...patch } : s)));
      } catch (err) {
        setToast({ kind: "err", text: String(err) });
        await reload();
      }
    },
    [reload],
  );

  const updateTags = useCallback(
    async (id: string, next: string[]) => {
      try {
        const clean = await setStickerTags(id, next);
        setStickers((prev) => prev.map((s) => (s.id === id ? { ...s, tags: clean } : s)));
        setTags(await listTags());
      } catch (err) {
        setToast({ kind: "err", text: String(err) });
      }
    },
    [],
  );

  // One sticker, straight from the right panel: always redoes the work.
  const describeOne = useCallback(
    async (id: string) => {
      setAiBusyId(id);
      try {
        const report = await aiDescribe([id]);
        if (report.failures.length > 0) {
          setToast({ kind: "err", text: report.failures[0].message });
        } else {
          const all = await listStickers("", [], "recent");
          const updated = all.find((s) => s.id === id);
          setToast({ kind: "ok", text: t("ai.oneDone", { name: updated?.name ?? "" }) });
          setAiPending(all.filter((s) => !s.aiAt));
        }
        await reload();
      } catch (err) {
        setToast({ kind: "err", text: String(err) });
      } finally {
        setAiBusyId(null);
      }
    },
    [reload, t],
  );

  const removeSelection = useCallback(
    async (ids: string[]) => {
      if (!ids.length) return;
      const ok = window.confirm(t("detail.deleteConfirm", { n: ids.length }));
      if (!ok) return;
      try {
        await deleteStickers(ids);
        setSelection([]);
        await reload();
      } catch (err) {
        setToast({ kind: "err", text: String(err) });
      }
    },
    [reload, t],
  );

  const toggleTag = useCallback((tag: string) => {
    setActiveTags((prev) => (prev.includes(tag) ? prev.filter((x) => x !== tag) : [...prev, tag]));
  }, []);

  const selectedSet = useMemo(() => new Set(selection), [selection]);

  const detailSticker = useMemo(() => {
    if (!selection.length) return null;
    const id = selection[selection.length - 1];
    return stickers.find((s) => s.id === id) ?? null;
  }, [selection, stickers]);

  const addSelectionToSheet = useCallback(
    (quantity: number) => {
      const chosen = stickers.filter((s) => selection.includes(s.id));
      if (!chosen.length) return;
      const added = chosen.flatMap((sticker) => copiesFromSticker(sticker, quantity));
      setSheetCopies((prev) => [...prev, ...added]);
      setView("sheet");
    },
    [selection, stickers],
  );

  // One entry point for adding: the library's quick button asks for one copy,
  // the sheet's picker asks for the quantity typed there.
  const addStickerCopies = useCallback(
    (sticker: Sticker, quantity: number) => {
      const added = copiesFromSticker(sticker, quantity);
      const total = sheetCopies.length + added.length;
      setSheetCopies((prev) => [...prev, ...added]);
      setToast({
        kind: "ok",
        text:
          added.length === 1
            ? t("sheet.addedOne", { name: sticker.name, total })
            : t("sheet.addedMany", { n: added.length, name: sticker.name, total }),
      });
    },
    [sheetCopies.length, t],
  );

  // Editing either dimension keeps the sticker's original proportions.
  const updateCopy = useCallback(
    (id: string, patch: Partial<Pick<SheetCopy, "widthMm" | "heightMm">>) => {
      setSheetCopies((prev) =>
        prev.map((copy) => (copy.id === id ? resizeCopy(copy, patch) : copy)),
      );
    },
    [],
  );

  const duplicateCopy = useCallback((id: string) => {
    setSheetCopies((prev) => {
      const index = prev.findIndex((c) => c.id === id);
      if (index < 0) return prev;
      const clone = { ...prev[index], id: newId() };
      return [...prev.slice(0, index + 1), clone, ...prev.slice(index + 1)];
    });
  }, []);

  const spinCopy = useCallback((id: string) => {
    setSheetCopies((prev) => prev.map((c) => (c.id === id ? rotateCopy(c) : c)));
  }, []);

  const removeCopy = useCallback((id: string) => {
    setSheetCopies((prev) => prev.filter((c) => c.id !== id));
  }, []);

  // Saving keeps the sheet editable: one .sns file with the artwork and the
  // layout, no PDF.
  const saveProject = useCallback(async () => {
    if (!sheetCopies.length) {
      setToast({ kind: "err", text: t("sheet.saveEmpty") });
      return;
    }
    try {
      const destination = await pickProjectFile(
        t("sheet.saveProjectTitle"),
        `hoja-stickers.${PROJECT_EXTENSION}`,
      );
      if (!destination) return;
      const saved = await saveSheetProject(destination, sheetSettings, sheetCopies);
      rememberLastSheet(saved);
      setToast({ kind: "ok", text: t("sheet.saved", { path: saved }) });
    } catch (err) {
      setToast({ kind: "err", text: String(err) });
    }
  }, [sheetCopies, sheetSettings, t]);

  // Replaces the sheet on screen. Anything already laid out is only dropped
  // after the user agrees, so a restore or an open never loses work silently.
  const loadProject = useCallback(
    async (path: string, announce: boolean) => {
      if (sheetCopies.length > 0 && !window.confirm(t("sheet.openConfirm"))) return;
      const project = await openSheetProject(path);
      setSheetCopies(project.copies);
      setSheetSettings(project.settings);
      setView("sheet");
      rememberLastSheet(project.convertedTo ?? path);
      if (!announce) return;
      setToast({
        kind: "ok",
        text: project.convertedTo
          ? t("sheet.loadedConverted", { n: project.copies.length, path: project.convertedTo })
          : t("sheet.loaded", { n: project.copies.length }),
      });
    },
    [sheetCopies.length, t],
  );

  // A `.sns` file, or a folder saved with the previous format (converted on open).
  const openProject = useCallback(
    async (source: "file" | "folder") => {
      try {
        const path =
          source === "file"
            ? await pickProjectToOpen(t("sheet.openProjectTitle"))
            : await pickProjectFolder(t("sheet.openFolderTitle"));
        if (!path) return;
        await loadProject(path, true);
      } catch (err) {
        setToast({ kind: "err", text: String(err) });
      }
    },
    [loadProject, t],
  );

  // Reopens the sheet from the last session as soon as the app starts.
  const restoredRef = useRef(false);
  useEffect(() => {
    if (!isTauri || restoredRef.current) return;
    restoredRef.current = true;
    const path = lastSheet();
    if (!path) return;
    void (async () => {
      try {
        await loadProject(path, false);
      } catch {
        // The file was moved or deleted: start with an empty sheet and stop
        // looking for it on the next launch.
        forgetLastSheet();
        setToast({ kind: "err", text: t("sheet.restoreFailed", { path }) });
      }
    })();
  }, [loadProject, t]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
      if (e.key === "Escape") setSelection([]);
      if (e.key === "Delete" && view === "library" && selection.length) {
        void removeSelection(selection);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selection, removeSelection, view]);

  const countLabel =
    info && info.count === 1 ? t("library.countOne") : t("library.count", { n: info?.count ?? 0 });

  return (
    <div className="app">
      <Sidebar
        view={view}
        onViewChange={setView}
        tags={tags}
        activeTags={activeTags}
        onToggleTag={toggleTag}
        onClearTags={() => setActiveTags([])}
        info={info}
        sheetCount={sheetCopies.length}
        onOpenFolder={() => void revealLibrary()}
        aiPending={aiPending.length}
        onOpenAi={() => setAiOpen(true)}
      />

      <main className="main">
        {view === "sheet" ? (
          <SheetView
            copies={sheetCopies}
            settings={sheetSettings}
            onSettings={(patch) => setSheetSettings((prev) => ({ ...prev, ...patch }))}
            onAddSticker={addStickerCopies}
            onGoLibrary={() => setView("library")}
            onUpdateCopy={updateCopy}
            onDuplicate={duplicateCopy}
            onRotate={spinCopy}
            onSaveProject={() => void saveProject()}
            onOpenProject={() => void openProject("file")}
            onOpenProjectFolder={() => void openProject("folder")}
            onRemove={removeCopy}
            onClear={() => setSheetCopies([])}
            onNotify={(text, kind) => setToast({ kind, text })}
          />
        ) : (
          <>
            <header className="toolbar">
              <div className="search">
                <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
                  <circle cx="11" cy="11" r="7" fill="none" stroke="currentColor" strokeWidth="2" />
                  <path d="M16.5 16.5L21 21" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
                </svg>
                <input
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder={t("search.placeholder")}
                  spellCheck={false}
                />
                {query && (
                  <button className="search-clear" onClick={() => setQuery("")} aria-label="limpiar">
                    ✕
                  </button>
                )}
              </div>

              <div className="toolbar-right">
                <span className="count-label">{countLabel}</span>
                <label className="sort">
                  <span>{t("sort.label")}</span>
                  <select value={sort} onChange={(e) => setSort(e.target.value as SortKey)}>
                    <option value="recent">{t("sort.recent")}</option>
                    <option value="name">{t("sort.name")}</option>
                    <option value="size">{t("sort.size")}</option>
                  </select>
                </label>
                <button className="primary-btn" onClick={handleImportClick} disabled={importing || !isTauri}>
                  {importing ? t("import.busy") : t("import.button")}
                </button>
              </div>
            </header>

            {selection.length > 0 && (
              <div className="selection-bar">
                <span>
                  {selection.length === 1
                    ? t("selection.n", { n: 1 })
                    : t("selection.n", { n: selection.length })}
                </span>
                <button className="ghost-btn" onClick={() => addSelectionToSheet(1)}>
                  {t("selection.addSheet", { n: selection.length })}
                </button>
                <button className="ghost-btn" onClick={() => void removeSelection(selection)}>
                  {t("selection.delete")}
                </button>
                <button className="link-btn" onClick={() => setSelection([])}>
                  {t("selection.clear")}
                </button>
              </div>
            )}

            {!isTauri && <div className="notice">{t("browser.warning")}</div>}

            {loading ? (
              <div className="placeholder">
                <span className="spinner" />
              </div>
            ) : stickers.length === 0 ? (
              <div className="placeholder">
                <div className="empty">
                  <div className="empty-art" aria-hidden="true">
                    <svg viewBox="0 0 120 90" width="120" height="90">
                      <rect x="12" y="14" width="42" height="42" rx="8" fill="none" stroke="currentColor" strokeWidth="2.5" strokeDasharray="6 6" opacity="0.5" />
                      <rect x="66" y="34" width="42" height="42" rx="8" fill="none" stroke="currentColor" strokeWidth="2.5" opacity="0.8" />
                      <path d="M78 55h18M87 46v18" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" opacity="0.8" />
                    </svg>
                  </div>
                  <h2>
                    {query || activeTags.length
                      ? t("library.emptyFiltered.title")
                      : t("library.empty.title")}
                  </h2>
                  <p>
                    {query || activeTags.length
                      ? t("library.emptyFiltered.body")
                      : t("library.empty.body")}
                  </p>
                  {!(query || activeTags.length) && (
                    <>
                      <button className="primary-btn" onClick={handleImportClick} disabled={!isTauri}>
                        {t("import.button")}
                      </button>
                      <span className="muted small">{t("import.formats")}</span>
                    </>
                  )}
                </div>
              </div>
            ) : (
              <StickerGrid
                items={stickers}
                selected={selectedSet}
                onSelect={handleSelect}
                onAdd={(sticker) => addStickerCopies(sticker, 1)}
              />
            )}
          </>
        )}

        {dropping && (
          <div className="drop-overlay">
            <div className="drop-card">{t("import.drop")}</div>
          </div>
        )}
      </main>

      {detailSticker && view === "library" && (
        <DetailPanel
          sticker={detailSticker}
          selectionCount={selection.length}
          onClose={() => setSelection([])}
          onPatch={patchSticker}
          onTags={updateTags}
          onDelete={(id) => void removeSelection(selection.length ? selection : [id])}
          onAi={(id) => void describeOne(id)}
          aiBusy={aiBusyId === detailSticker.id}
        />
      )}

      <AiPanel
        open={aiOpen}
        pending={aiPending}
        total={info?.count ?? 0}
        onClose={() => setAiOpen(false)}
        onUpdated={async () => {
          await reload();
          await refreshPending();
        }}
      />

      {toast && <div className={`toast ${toast.kind}`}>{toast.text}</div>}
    </div>
  );
}
