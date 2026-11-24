import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FacetCount, LibraryInfo, SortKey, Sticker, TagCount } from "./types";
import {
  PROJECT_EXTENSION,
  aiClassify,
  aiDescribe,
  deleteStickers,
  exportLibraryArchive,
  importLibraryArchive,
  inspectLibraryArchive,
  pickLibraryArchive,
  pickLibraryExport,
  type ArchiveAction,
  type ArchiveConflict,
  importPaths,
  isTauri,
  libraryInfo,
  recalcDpi,
  listSeries,
  listStickers,
  listTags,
  listTipos,
  mergeSeries,
  deleteSavedSheet,
  listSavedSheets,
  openSavedSheet,
  openSheetProject,
  renameSavedSheet,
  saveSavedSheet,
  type SavedSheet,
  pickImages,
  pickProjectFile,
  pickProjectFolder,
  pickProjectToOpen,
  revealLibrary,
  saveSheetProject,
  setStickerClassification,
  setStickersClassification,
  setStickerTags,
  updateSticker,
} from "./lib/api";
import { useI18n } from "./lib/i18n";
import { filterStickers, parseQuery } from "./lib/search";
import { groupStickers, visualOrder, type GroupBy } from "./lib/group";
import {
  forgetLastLocalSheet,
  forgetLastSheet,
  lastLocalSheet,
  lastSheet,
  rememberLastLocalSheet,
  rememberLastSheet,
} from "./lib/recent";
import {
  DEFAULT_SETTINGS,
  copiesFromSticker,
  newId,
  resizeCopiesToArea,
  resizeCopy,
  rotateCopy,
  sheetFileName,
  type SheetCopy,
  type SheetSettings,
} from "./lib/sheet";
import { Sidebar } from "./components/Sidebar";
import { StickerGrid } from "./components/StickerGrid";
import { DetailPanel } from "./components/DetailPanel";
import { SheetView } from "./components/SheetView";
import { SavedSheetsPanel } from "./components/SavedSheetsPanel";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { AiPanel } from "./components/AiPanel";
import { SeriesPanel } from "./components/SeriesPanel";
import { LibraryArchivePanel } from "./components/LibraryArchivePanel";
import { BulkClassification } from "./components/BulkClassification";

interface Toast {
  kind: "ok" | "err";
  text: string;
}

interface CopySize {
  id: string;
  widthMm: number;
  heightMm: number;
}

export default function App() {
  const { t } = useI18n();
  const [view, setView] = useState<"library" | "sheet">("library");
  const [all, setAll] = useState<Sticker[]>([]);
  const [tags, setTags] = useState<TagCount[]>([]);
  const [info, setInfo] = useState<LibraryInfo | null>(null);
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<SortKey>("recent");
  const [activeTags, setActiveTags] = useState<string[]>([]);
  const [series, setSeries] = useState<FacetCount[]>([]);
  const [tipos, setTipos] = useState<FacetCount[]>([]);
  const [activeSerie, setActiveSerie] = useState<string | null>(null);
  const [activeTipo, setActiveTipo] = useState<string | null>(null);
  const [groupBy, setGroupBy] = useState<GroupBy | "none">("none");
  const [selection, setSelection] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [importing, setImporting] = useState(false);
  const [dropping, setDropping] = useState(false);
  const [toast, setToast] = useState<Toast | null>(null);
  const [sheetCopies, setSheetCopies] = useState<SheetCopy[]>([]);
  // Sizes each copy had before the last bulk area change, so one click can
  // put them back.
  const [areaUndo, setAreaUndo] = useState<CopySize[] | null>(null);
  const [sheetSettings, setSheetSettings] = useState<SheetSettings>(DEFAULT_SETTINGS);
  // Sheets kept in the library: the id is set once saved, so saving again
  // rewrites the same one instead of piling up copies.
  const [savedSheets, setSavedSheets] = useState<SavedSheet[]>([]);
  const [sheetId, setSheetId] = useState<string | null>(null);
  const [sheetName, setSheetName] = useState("");
  const [sheetsPanel, setSheetsPanel] = useState<"open" | "save" | null>(null);
  // A question drawn by the app itself: the webview's own confirm returns
  // before the user answers, which let actions run without a yes.
  const [ask, setAsk] = useState<{ message: string; resolve: (value: boolean) => void } | null>(
    null,
  );
  const [sheetsBusy, setSheetsBusy] = useState(false);
  const [aiOpen, setAiOpen] = useState(false);
  const [seriesOpen, setSeriesOpen] = useState(false);
  const [merging, setMerging] = useState(false);
  const [archiveBusy, setArchiveBusy] = useState(false);
  const [recalcBusy, setRecalcBusy] = useState(false);
  const [archivePeek, setArchivePeek] = useState<{
    path: string;
    total: number;
    conflicts: ArchiveConflict[];
  } | null>(null);
  const [aiPending, setAiPending] = useState<Sticker[]>([]);
  const [aiBusyId, setAiBusyId] = useState<string | null>(null);
  const anchorRef = useRef<string | null>(null);

  // The library sits in memory: typing filters it without another SQLite trip.
  const parsed = useMemo(() => parseQuery(query), [query]);
  const stickers = useMemo(
    () => filterStickers(all, parsed, { tags: activeTags, serie: activeSerie, tipo: activeTipo }),
    [all, parsed, activeTags, activeSerie, activeTipo],
  );
  // Grouping reorders what is painted, so shift-click walks the visual order.
  const groups = useMemo(() => {
    if (groupBy === "none") return [{ key: "", label: "", items: stickers }];
    return groupStickers(stickers, groupBy).map((group) => ({
      key: group.key,
      items: group.items,
      label: group.key || (groupBy === "serie" ? t("facet.noSerie") : t("facet.noTipo")),
    }));
  }, [stickers, groupBy, t]);
  const ordered = useMemo(() => visualOrder(groups), [groups]);

  const reload = useCallback(async () => {
    if (!isTauri) {
      setLoading(false);
      return;
    }
    try {
      const [list, tagList, serieList, tipoList, lib] = await Promise.all([
        listStickers("", [], sort),
        listTags(),
        listSeries(),
        listTipos(),
        libraryInfo(),
      ]);
      setAll(list);
      setTags(tagList);
      setSeries(serieList);
      setTipos(tipoList);
      setInfo(lib);
    } catch (err) {
      setToast({ kind: "err", text: String(err) });
    } finally {
      setLoading(false);
    }
  }, [sort]);

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

  // Stickers imported before the density was read still carry the 300 dpi
  // assumption; this asks the files what they really declare.
  const recalcDensities = useCallback(async () => {
    if (!isTauri || recalcBusy) return;
    setRecalcBusy(true);
    try {
      const report = await recalcDpi();
      await reload();
      if (report.failures.length > 0) {
        setToast({ kind: "err", text: t("library.recalcFailed", { n: report.failures.length }) });
        return;
      }
      const fixed = report.updated + report.keptSize;
      const done = t("library.recalcDone", { updated: fixed, checked: report.checked });
      setToast({
        kind: "ok",
        text:
          report.withoutDensity > 0
            ? `${done} · ${t("library.recalcNoDensity", { n: report.withoutDensity })}`
            : done,
      });
    } catch (err) {
      setToast({ kind: "err", text: String(err) });
    } finally {
      setRecalcBusy(false);
    }
  }, [recalcBusy, reload, t]);

  useEffect(() => {
    if (!toast) return;
    const id = window.setTimeout(() => setToast(null), 4200);
    return () => window.clearTimeout(id);
  }, [toast]);

  const confirmAction = useCallback(
    (message: string) => new Promise<boolean>((resolve) => setAsk({ message, resolve })),
    [],
  );

  const answerConfirm = useCallback(
    (value: boolean) => {
      ask?.resolve(value);
      setAsk(null);
    },
    [ask],
  );

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
          const a = ordered.findIndex((s) => s.id === anchorRef.current);
          const b = ordered.findIndex((s) => s.id === id);
          if (a >= 0 && b >= 0) {
            const [from, to] = a < b ? [a, b] : [b, a];
            return ordered.slice(from, to + 1).map((s) => s.id);
          }
        }
        anchorRef.current = id;
        return [id];
      });
    },
    [ordered],
  );

  const patchSticker = useCallback(
    async (id: string, patch: { name?: string; widthMm?: number; heightMm?: number }) => {
      try {
        await updateSticker(id, patch);
        setAll((prev) => prev.map((s) => (s.id === id ? { ...s, ...patch } : s)));
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
        setAll((prev) => prev.map((s) => (s.id === id ? { ...s, tags: clean } : s)));
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

  const patchClassification = useCallback(
    async (id: string, serie: string, tipo: string) => {
      try {
        await setStickerClassification(id, serie, tipo);
        setAll((prev) => prev.map((s) => (s.id === id ? { ...s, serie, tipo } : s)));
        setSeries(await listSeries());
        setTipos(await listTipos());
      } catch (err) {
        setToast({ kind: "err", text: String(err) });
      }
    },
    [],
  );

  // One serie or type for the whole selection, from the bar above the grid.
  const classifySelection = useCallback(
    async (patch: { serie?: string; tipo?: string }) => {
      if (!selection.length) return;
      try {
        const changed = await setStickersClassification(selection, patch);
        // One reload keeps cards and both facet lists in step with SQLite.
        await reload();
        setToast({ kind: "ok", text: t("selection.applied", { n: changed }) });
      } catch (err) {
        setToast({ kind: "err", text: String(err) });
      }
    },
    [reload, selection, t],
  );

  // Folding series into one name; the facet that was active follows the merge.
  const mergeInto = useCallback(
    async (from: string[], into: string) => {
      setMerging(true);
      try {
        const moved = await mergeSeries(from, into);
        const name = into.trim();
        if (activeSerie !== null && from.includes(activeSerie)) setActiveSerie(name);
        await reload();
        setToast({ kind: "ok", text: t("series.merged", { n: moved, name }) });
      } catch (err) {
        setToast({ kind: "err", text: String(err) });
      } finally {
        setMerging(false);
      }
    },
    [activeSerie, reload, t],
  );

  // The whole library travels as one .zip, artwork and manifest.
  const exportArchive = useCallback(async () => {
    try {
      const destination = await pickLibraryExport(
        t("library.exportTitle"),
        "biblioteca-stickers.zip",
      );
      if (!destination) return;
      setArchiveBusy(true);
      const saved = await exportLibraryArchive(destination);
      setToast({ kind: "ok", text: t("library.exported", { path: saved }) });
    } catch (err) {
      setToast({ kind: "err", text: String(err) });
    } finally {
      setArchiveBusy(false);
    }
  }, [t]);

  const runArchiveImport = useCallback(
    async (path: string, decisions: { id: string; action: ArchiveAction }[]) => {
      setArchiveBusy(true);
      try {
        const report = await importLibraryArchive(path, decisions);
        await reload();
        await refreshPending();
        setArchivePeek(null);
        if (report.failures.length > 0) {
          setToast({
            kind: "err",
            text: t("archive.failed", {
              n: report.failures.length,
              message: report.failures[0].message,
            }),
          });
        } else {
          setToast({
            kind: "ok",
            text: t("archive.done", {
              imported: report.imported,
              replaced: report.replaced,
              duplicated: report.duplicated,
              skipped: report.skipped,
            }),
          });
        }
      } catch (err) {
        setToast({ kind: "err", text: String(err) });
      } finally {
        setArchiveBusy(false);
      }
    },
    [reload, refreshPending, t],
  );

  // With no collisions the archive goes in straight away; otherwise the window
  // asks sticker by sticker before writing anything.
  const startArchiveImport = useCallback(async () => {
    let path: string | null = null;
    try {
      path = await pickLibraryArchive(t("library.importTitle"));
    } catch (err) {
      setToast({ kind: "err", text: String(err) });
      return;
    }
    if (!path) return;
    setArchiveBusy(true);
    try {
      const peek = await inspectLibraryArchive(path);
      if (peek.conflicts.length === 0) {
        await runArchiveImport(path, []);
      } else {
        setArchivePeek({ path, total: peek.total, conflicts: peek.conflicts });
      }
    } catch (err) {
      setToast({ kind: "err", text: String(err) });
    } finally {
      setArchiveBusy(false);
    }
  }, [runArchiveImport, t]);

  // Text-only classification of the whole library: names and tags stay as they are.
  const classifyStickers = useCallback(async () => {
    try {
      const report = await aiClassify();
      await reload();
      if (report.updated.length > 0) {
        setToast({ kind: "ok", text: t("ai.classifyDone", { n: report.updated.length }) });
      } else if (report.failures.length > 0) {
        setToast({
          kind: "err",
          text: t("ai.classifyFailed", { message: report.failures[0].message }),
        });
      } else {
        setToast({ kind: "ok", text: t("ai.classifyNone") });
      }
    } catch (err) {
      setToast({ kind: "err", text: t("ai.classifyFailed", { message: String(err) }) });
    }
  }, [reload, t]);

  const removeSelection = useCallback(
    async (ids: string[]) => {
      if (!ids.length) return;
      const ok = await confirmAction(t("detail.deleteConfirm", { n: ids.length }));
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
      const added = chosen.flatMap((sticker) =>
        copiesFromSticker(sticker, quantity, sheetSettings.areaCm2),
      );
      setSheetCopies((prev) => [...prev, ...added]);
      setView("sheet");
    },
    [selection, sheetSettings.areaCm2, stickers],
  );

  // One entry point for adding: the library's quick button asks for one copy,
  // the sheet's picker asks for the quantity typed there.
  const addStickerCopies = useCallback(
    (sticker: Sticker, quantity: number) => {
      const added = copiesFromSticker(sticker, quantity, sheetSettings.areaCm2);
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
    [sheetCopies.length, sheetSettings.areaCm2, t],
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

  // Evens out what is already on the sheet, so every piece ends up the same
  // size as the one a new sticker would get. The previous sizes are kept once.
  const applyAreaToCopies = useCallback(() => {
    const area = sheetSettings.areaCm2;
    if (!(area > 0) || sheetCopies.length === 0) return;
    setAreaUndo(
      sheetCopies.map((copy) => ({
        id: copy.id,
        widthMm: copy.widthMm,
        heightMm: copy.heightMm,
      })),
    );
    setSheetCopies((prev) => resizeCopiesToArea(prev, area));
    setToast({ kind: "ok", text: t("sheet.areaApplied", { n: sheetCopies.length, area }) });
  }, [sheetCopies, sheetSettings.areaCm2, t]);

  // Only the sizes go back: copies added or removed since keep their place.
  const undoArea = useCallback(() => {
    if (!areaUndo) return;
    const before = new Map(areaUndo.map((size) => [size.id, size]));
    setSheetCopies((prev) =>
      prev.map((copy) => {
        const size = before.get(copy.id);
        return size
          ? { ...copy, widthMm: size.widthMm, heightMm: size.heightMm }
          : copy;
      }),
    );
    setAreaUndo(null);
    setToast({ kind: "ok", text: t("sheet.areaUndone") });
  }, [areaUndo, t]);

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
        sheetFileName(sheetName, "hoja-stickers", PROJECT_EXTENSION),
      );
      if (!destination) return;
      const saved = await saveSheetProject(destination, sheetSettings, sheetCopies);
      rememberLastSheet(saved);
      setToast({ kind: "ok", text: t("sheet.saved", { path: saved }) });
    } catch (err) {
      setToast({ kind: "err", text: String(err) });
    }
  }, [sheetCopies, sheetSettings, sheetName, t]);

  const refreshSheets = useCallback(async () => {
    if (!isTauri) return;
    try {
      setSavedSheets(await listSavedSheets());
    } catch {
      // The list is only a shortcut; failing to load it never blocks the sheet.
    }
  }, []);

  useEffect(() => {
    void refreshSheets();
  }, [refreshSheets]);

  // The sheet keeps the names it was saved with; show today's library names
  // instead, so a copy list never reads like «path8424».
  const withLibraryNames = useCallback(async (copies: SheetCopy[]) => {
    try {
      const all = await listStickers("", [], "recent");
      const names = new Map(all.map((s) => [s.id, s.name]));
      return copies.map((copy) => {
        const name = names.get(copy.stickerId);
        return name && name !== copy.name ? { ...copy, name } : copy;
      });
    } catch {
      // The library is not ready; the saved names are still usable.
      return copies;
    }
  }, []);

  // Replaces the sheet on screen. Anything already laid out is only dropped
  // after the user agrees, so a restore or an open never loses work silently.
  const loadProject = useCallback(
    async (path: string, announce: boolean) => {
      if (sheetCopies.length > 0 && !(await confirmAction(t("sheet.openConfirm")))) return;
      const project = await openSheetProject(path);
      const copies = await withLibraryNames(project.copies);
      setSheetId(null);
      setSheetName("");
      setAreaUndo(null);
      setSheetCopies(copies);
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
    [sheetCopies.length, t, withLibraryNames],
  );

  // Saves into the library database: nothing leaves the app and the sheet is
  // still editable next time it is opened.
  const saveLocalSheet = useCallback(
    async (name: string) => {
      if (!sheetCopies.length) {
        setToast({ kind: "err", text: t("sheet.saveEmpty") });
        return;
      }
      setSheetsBusy(true);
      try {
        const saved = await saveSavedSheet(sheetId, name, sheetSettings, sheetCopies);
        setSheetId(saved.id);
        setSheetName(saved.name);
        rememberLastLocalSheet(saved.id);
        setSheetsPanel(null);
        await refreshSheets();
        setToast({ kind: "ok", text: t("sheet.savedLocal", { name: saved.name }) });
      } catch (err) {
        setToast({ kind: "err", text: String(err) });
      } finally {
        setSheetsBusy(false);
      }
    },
    [refreshSheets, sheetCopies, sheetId, sheetSettings, t],
  );

  // An already saved sheet is overwritten in place; a new one asks for a name.
  const beginSaveLocal = useCallback(() => {
    if (!sheetCopies.length) {
      setToast({ kind: "err", text: t("sheet.saveEmpty") });
      return;
    }
    if (sheetId) void saveLocalSheet(sheetName);
    else setSheetsPanel("save");
  }, [saveLocalSheet, sheetCopies.length, sheetId, sheetName, t]);

  const openLocalSheet = useCallback(
    async (sheet: SavedSheet) => {
      if (sheetCopies.length > 0 && !(await confirmAction(t("sheet.openConfirm")))) return;
      setSheetsBusy(true);
      try {
        const project = await openSavedSheet(sheet.id);
        const copies = await withLibraryNames(project.copies);
        setAreaUndo(null);
        setSheetCopies(copies);
        setSheetSettings(project.settings);
        setSheetId(sheet.id);
        setSheetName(sheet.name);
        rememberLastLocalSheet(sheet.id);
        setSheetsPanel(null);
        setView("sheet");
        setToast({
          kind: "ok",
          text: t("sheet.openedLocal", { name: sheet.name, n: copies.length }),
        });
      } catch (err) {
        setToast({ kind: "err", text: String(err) });
      } finally {
        setSheetsBusy(false);
      }
    },
    [sheetCopies.length, t, withLibraryNames],
  );

  // Renaming touches the name only: the layout inside keeps its place.
  const renameLocalSheet = useCallback(
    async (sheet: SavedSheet, name: string) => {
      const clean = name.trim();
      if (!clean || clean === sheet.name) return;
      try {
        const renamed = await renameSavedSheet(sheet.id, clean);
        if (sheetId === renamed.id) setSheetName(renamed.name);
        await refreshSheets();
        setToast({ kind: "ok", text: t("sheet.renamed", { name: renamed.name }) });
      } catch (err) {
        setToast({ kind: "err", text: String(err) });
      }
    },
    [refreshSheets, sheetId, t],
  );

  // A new sheet: an empty workspace with no name yet, saved when asked.
  const newLocalSheet = useCallback(async () => {
    if (sheetCopies.length > 0 && !(await confirmAction(t("sheet.newSheetConfirm")))) return;
    setSheetCopies([]);
    setAreaUndo(null);
    setSheetId(null);
    setSheetName("");
    forgetLastLocalSheet();
    setSheetsPanel(null);
    setView("sheet");
    setToast({ kind: "ok", text: t("sheet.newSheetDone") });
  }, [confirmAction, sheetCopies.length, t]);

  const removeLocalSheet = useCallback(
    async (sheet: SavedSheet) => {
      if (!(await confirmAction(t("sheet.sheetsDeleteConfirm", { name: sheet.name })))) return;
      setSheetsBusy(true);
      try {
        await deleteSavedSheet(sheet.id);
        if (sheetId === sheet.id) {
          setSheetId(null);
          setSheetName("");
        }
        if (lastLocalSheet() === sheet.id) forgetLastLocalSheet();
        await refreshSheets();
        setToast({ kind: "ok", text: t("sheet.sheetDeleted", { name: sheet.name }) });
      } catch (err) {
        setToast({ kind: "err", text: String(err) });
      } finally {
        setSheetsBusy(false);
      }
    },
    [refreshSheets, sheetId, t],
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

  // Reopens the sheet from the last session as soon as the app starts: the one
  // saved inside the app or a `.sns` file, whichever was used last.
  const restoredRef = useRef(false);
  useEffect(() => {
    if (!isTauri || restoredRef.current) return;
    restoredRef.current = true;
    const localId = lastLocalSheet();
    const path = lastSheet();
    if (!localId && !path) return;
    void (async () => {
      if (localId) {
        try {
          const project = await openSavedSheet(localId);
          const sheets = await listSavedSheets();
          const sheet = sheets.find((item) => item.id === localId);
          if (!sheet) throw new Error("La hoja ya no está guardada");
          setSavedSheets(sheets);
          setAreaUndo(null);
          setSheetCopies(await withLibraryNames(project.copies));
          setSheetSettings(project.settings);
          setSheetId(sheet.id);
          setSheetName(sheet.name);
          setView("sheet");
          return;
        } catch {
          // Deleted or unreadable: fall back to a `.sns` from before, if any.
          forgetLastLocalSheet();
        }
      }
      if (!path) return;
      try {
        await loadProject(path, false);
      } catch {
        // The file was moved or deleted: start with an empty sheet and stop
        // looking for it on the next launch.
        forgetLastSheet();
        setToast({ kind: "err", text: t("sheet.restoreFailed", { path }) });
      }
    })();
  }, [loadProject, t, withLibraryNames]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // While a question is open, Escape belongs to it.
      if (ask) return;
      const target = e.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
      if (e.key === "Escape") setSelection([]);
      if (e.key === "Delete" && view === "library" && selection.length) {
        void removeSelection(selection);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ask, selection, removeSelection, view]);

  const filtering =
    parsed.active || activeTags.length > 0 || activeSerie !== null || activeTipo !== null;
  const countLabel = filtering
    ? t("library.countFiltered", { n: stickers.length, total: info?.count ?? 0 })
    : info && info.count === 1
      ? t("library.countOne")
      : t("library.count", { n: info?.count ?? 0 });

  return (
    <div className="app">
      <Sidebar
        view={view}
        onViewChange={setView}
        tags={tags}
        activeTags={activeTags}
        onToggleTag={toggleTag}
        onClearTags={() => setActiveTags([])}
        series={series}
        tipos={tipos}
        activeSerie={activeSerie}
        activeTipo={activeTipo}
        onSelectSerie={setActiveSerie}
        onSelectTipo={setActiveTipo}
        onReviewSeries={() => setSeriesOpen(true)}
        info={info}
        sheetCount={sheetCopies.length}
        onOpenFolder={() => void revealLibrary()}
        onRecalcDpi={() => void recalcDensities()}
        recalcBusy={recalcBusy}
        onExportLibrary={() => void exportArchive()}
        onImportLibrary={() => void startArchiveImport()}
        archiveBusy={archiveBusy}
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
            onApplyArea={applyAreaToCopies}
            onUndoArea={undoArea}
            canUndoArea={areaUndo !== null}
            sheetName={sheetName}
            onConfirm={confirmAction}
            onSaveLocal={beginSaveLocal}
            onOpenSheets={() => {
              setSheetsPanel("open");
              void refreshSheets();
            }}
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
                  title={t("search.hint")}
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
                  <span>{t("group.label")}</span>
                  <select
                    value={groupBy}
                    onChange={(e) => setGroupBy(e.target.value as GroupBy | "none")}
                  >
                    <option value="none">{t("group.none")}</option>
                    <option value="serie">{t("group.serie")}</option>
                    <option value="tipo">{t("group.tipo")}</option>
                  </select>
                </label>
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
                    ? t("selection.one")
                    : t("selection.n", { n: selection.length })}
                </span>
                <BulkClassification
                  series={series}
                  onApply={(patch) => void classifySelection(patch)}
                />
                <div className="selection-actions">
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
                    {filtering ? t("library.emptyFiltered.title") : t("library.empty.title")}
                  </h2>
                  <p>
                    {filtering ? t("library.emptyFiltered.body") : t("library.empty.body")}
                  </p>
                  {!filtering && (
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
                groups={groups}
                showHeaders={groupBy !== "none"}
                selected={selectedSet}
                query={parsed.active ? parsed : undefined}
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
          series={series}
          onClassification={patchClassification}
          onDelete={(id) => void removeSelection(selection.length ? selection : [id])}
          onAi={(id) => void describeOne(id)}
          aiBusy={aiBusyId === detailSticker.id}
        />
      )}

      {sheetsPanel && (
        <SavedSheetsPanel
          mode={sheetsPanel}
          sheets={savedSheets}
          defaultName={sheetName || t("sheet.sheetNamePlaceholder")}
          busy={sheetsBusy}
          onClose={() => setSheetsPanel(null)}
          onSave={(name) => void saveLocalSheet(name)}
          onOpen={(sheet) => void openLocalSheet(sheet)}
          onDelete={(sheet) => void removeLocalSheet(sheet)}
          onRename={(sheet, name) => void renameLocalSheet(sheet, name)}
          onNew={() => void newLocalSheet()}
        />
      )}

      {archivePeek && (
        <LibraryArchivePanel
          total={archivePeek.total}
          conflicts={archivePeek.conflicts}
          busy={archiveBusy}
          onClose={() => setArchivePeek(null)}
          onImport={(decisions) => void runArchiveImport(archivePeek.path, decisions)}
        />
      )}

      {seriesOpen && (
        <SeriesPanel
          series={series}
          busy={merging}
          onClose={() => setSeriesOpen(false)}
          onMerge={(from, into) => void mergeInto(from, into)}
        />
      )}

      <AiPanel
        open={aiOpen}
        pending={aiPending}
        total={info?.count ?? 0}
        onClose={() => setAiOpen(false)}
        unclassified={all.filter((s) => !s.classifiedAt).length}
        onClassify={classifyStickers}
        onUpdated={async () => {
          await reload();
          await refreshPending();
        }}
      />

      {ask && <ConfirmDialog message={ask.message} onAnswer={answerConfirm} />}

      {toast && <div className={`toast ${toast.kind}`}>{toast.text}</div>}
    </div>
  );
}
