import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  AiKeyCheck,
  AiReport,
  AiStatus,
  FacetCount,
  ImportProgress,
  ImportReport,
  LibraryInfo,
  SortKey,
  MergeGroup,
  Sticker,
  TagCount,
} from "../types";
import type { SheetCopy, SheetSettings } from "./sheet";
import { hydrateProject, projectCopies, type LoadedProject, type LoadedProjectPayload } from "./project";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export function assetUrl(path: string): string {
  if (!path) return "";
  if (!isTauri) return "";
  return convertFileSrc(path);
}

export function importPaths(paths: string[], allowDuplicates = false): Promise<ImportReport> {
  return invoke<ImportReport>("import_paths", { paths, allowDuplicates });
}

/** Stops the running import after the file it is working on. */
export function cancelImport(): Promise<void> {
  return invoke<void>("cancel_import");
}

/** Progress of the import: one event per file, off the UI thread. */
export function onImportProgress(
  handler: (progress: ImportProgress) => void,
): Promise<() => void> {
  return listen<ImportProgress>("import:progress", (event) => handler(event.payload));
}

export function listStickers(query: string, tags: string[], sort: SortKey): Promise<Sticker[]> {
  return invoke<Sticker[]>("list_stickers", { query, tags, sort });
}

export function listTags(): Promise<TagCount[]> {
  return invoke<TagCount[]>("list_tags");
}

export function listSeries(): Promise<FacetCount[]> {
  return invoke<FacetCount[]>("list_series");
}

export function listTipos(): Promise<FacetCount[]> {
  return invoke<FacetCount[]>("list_tipos");
}

/** Manual edit from the detail panel: "" clears the field. */
export function setStickerClassification(
  id: string,
  serie: string,
  tipo: string,
): Promise<void> {
  return invoke("set_sticker_classification", { id, serie, tipo });
}

/** Bulk edit from the selection bar: only the fields given are written. */
export function setStickersClassification(
  ids: string[],
  patch: { serie?: string; tipo?: string },
): Promise<number> {
  return invoke<number>("set_stickers_classification", {
    ids,
    serie: patch.serie ?? null,
    tipo: patch.tipo ?? null,
  });
}

/** Every sticker of `from` ends up under `into`; returns how many moved. */
export function mergeSeries(from: string[], into: string): Promise<number> {
  return invoke<number>("merge_series", { from, into });
}

/** Text-only pass: which tags in the library mean the same. Writes nothing. */
export function aiTagSuggestions(): Promise<MergeGroup[]> {
  return invoke<MergeGroup[]>("ai_normalize_tags");
}

/** Text-only pass: which series names are the same work. Writes nothing. */
export function aiSeriesSuggestions(): Promise<MergeGroup[]> {
  return invoke<MergeGroup[]>("ai_series_suggestions");
}

/** Folds every sticker tagged with `from` into `into`; returns how many changed. */
export function mergeTags(from: string[], into: string): Promise<number> {
  return invoke<number>("merge_tags", { from, into });
}

/** One sticker of the archive that already exists here. */
export interface ArchiveConflict {
  id: string;
  name: string;
  existingName: string;
}

export interface ArchivePeek {
  total: number;
  conflicts: ArchiveConflict[];
}

export interface ArchiveReport {
  imported: number;
  replaced: number;
  duplicated: number;
  skipped: number;
  failures: { name: string; message: string }[];
}

/** What to do with a colliding id: «keep» this one, «replace» it, or keep both. */
export type ArchiveAction = "keep" | "replace" | "both";

/** Writes every sticker and its artwork into one `.zip`. */
export function exportLibraryArchive(destination: string): Promise<string> {
  return invoke<string>("export_library", { destination });
}

/** Lists the archive's stickers that this library already has. Writes nothing. */
export function inspectLibraryArchive(path: string): Promise<ArchivePeek> {
  return invoke<ArchivePeek>("inspect_library_archive", { path });
}

export function importLibraryArchive(
  path: string,
  decisions: { id: string; action: ArchiveAction }[],
): Promise<ArchiveReport> {
  return invoke<ArchiveReport>("import_library_archive", { path, decisions });
}

export async function pickLibraryExport(
  title: string,
  defaultName: string,
): Promise<string | null> {
  const picked = await save({
    title,
    defaultPath: defaultName,
    filters: [{ name: "Biblioteca (.zip)", extensions: ["zip"] }],
  });
  if (typeof picked !== "string" || !picked) return null;
  return /\.zip$/i.test(picked) ? picked : `${picked}.zip`;
}

export async function pickLibraryArchive(title: string): Promise<string | null> {
  const picked = await open({
    multiple: false,
    title,
    filters: [{ name: "Biblioteca (.zip)", extensions: ["zip"] }],
  });
  return typeof picked === "string" && picked ? picked : null;
}

export function updateSticker(
  id: string,
  patch: { name?: string; widthMm?: number; heightMm?: number },
): Promise<void> {
  return invoke("update_sticker", {
    id,
    name: patch.name ?? null,
    widthMm: patch.widthMm ?? null,
    heightMm: patch.heightMm ?? null,
  });
}

export function setStickerTags(id: string, tags: string[]): Promise<string[]> {
  return invoke<string[]>("set_sticker_tags", { id, tags });
}

/** The Groq keys live in Rust: the webview only learns their names and tails. */
export function aiStatus(): Promise<AiStatus> {
  return invoke<AiStatus>("ai_status");
}

export function aiAddKey(name: string, key: string): Promise<AiStatus> {
  return invoke<AiStatus>("ai_add_key", { name, key });
}

export function aiUpdateKey(
  id: string,
  patch: { name?: string; key?: string },
): Promise<AiStatus> {
  return invoke<AiStatus>("ai_update_key", {
    id,
    name: patch.name ?? null,
    key: patch.key ?? null,
  });
}

export function aiRemoveKey(id: string): Promise<AiStatus> {
  return invoke<AiStatus>("ai_remove_key", { id });
}

/** The first id is used first when the AI runs. */
export function aiReorderKeys(ids: string[]): Promise<AiStatus> {
  return invoke<AiStatus>("ai_reorder_keys", { ids });
}

/** A single cheap request, so one key can be checked on its own. */
export function aiTestKey(id: string): Promise<AiKeyCheck> {
  return invoke<AiKeyCheck>("ai_test_key", { id });
}

/** Fills serie and tipo from names and tags only: no image, no rewrite of names. */
export function aiClassify(): Promise<AiReport> {
  return invoke<AiReport>("ai_classify");
}

export function aiDescribe(ids: string[]): Promise<AiReport> {
  return invoke<AiReport>("ai_describe", { ids });
}

export function deleteStickers(ids: string[]): Promise<number> {
  return invoke<number>("delete_stickers", { ids });
}

export function libraryInfo(): Promise<LibraryInfo> {
  return invoke<LibraryInfo>("library_info");
}

export function revealLibrary(): Promise<void> {
  return invoke("reveal_library");
}

/** What a density pass found across the library. */
export interface DpiReport {
  checked: number;
  /** Size recomputed from the pixels and the newly read density. */
  updated: number;
  /** Hand-sized stickers: only their density changed. */
  keptSize: number;
  unchanged: number;
  /** The file declares no density at all (WebP, PNG/JPG without metadata). */
  withoutDensity: number;
  failures: { name: string; message: string }[];
}

/** Reads the DPI of the already imported artwork again. */
export function recalcDpi(): Promise<DpiReport> {
  return invoke<DpiReport>("recalc_dpi");
}

export interface PdfExportItem {
  path: string;
  /** Top-left of the footprint on the page, the cut line included. */
  xMm: number;
  yMm: number;
  /** Degrees clockwise: a multiple of 45. */
  rotation: number;
  /** Artwork box inside that footprint, before rotating. */
  artXMm: number;
  artYMm: number;
  artWMm: number;
  artHMm: number;
}

export interface PdfExportPage {
  widthMm: number;
  heightMm: number;
  items: PdfExportItem[];
}

export async function pickPdfPath(defaultName: string): Promise<string | null> {
  const picked = await save({
    title: "Exportar PDF",
    defaultPath: defaultName,
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  if (typeof picked !== "string" || !picked) return null;
  return /\.pdf$/i.test(picked) ? picked : `${picked}.pdf`;
}

export function exportSheetPdf(
  path: string,
  pages: PdfExportPage[],
  cutLines: boolean,
  outlineWidthMm: number,
  outlinePaddingMm: number,
): Promise<string> {
  return invoke<string>("export_sheet_pdf", {
    path,
    pages,
    cutLines,
    outlineWidthMm,
    outlinePaddingMm,
  });
}

/** The app's own sheet format: a single file, not a PDF. */
export const PROJECT_EXTENSION = "sns";

const PROJECT_FILTER = [
  { name: "Hoja de stickers", extensions: [PROJECT_EXTENSION] },
  { name: "Todos los archivos", extensions: ["*"] },
];

export async function pickProjectFile(title: string, defaultName: string): Promise<string | null> {
  const picked = await save({ title, defaultPath: defaultName, filters: PROJECT_FILTER });
  if (typeof picked !== "string" || !picked) return null;
  return new RegExp(`\\.${PROJECT_EXTENSION}$`, "i").test(picked)
    ? picked
    : `${picked}.${PROJECT_EXTENSION}`;
}

export async function pickProjectToOpen(title: string): Promise<string | null> {
  const picked = await open({ multiple: false, title, filters: PROJECT_FILTER });
  return typeof picked === "string" && picked ? picked : null;
}

/** Sheets saved with the previous format were folders, not single files. */
export async function pickProjectFolder(title: string): Promise<string | null> {
  const picked = await open({ directory: true, multiple: false, title });
  return typeof picked === "string" && picked ? picked : null;
}

/**
 * Saves the sheet as one `.sns` file: the artwork it uses plus `hoja.json` with
 * the layout inside a ZIP container, so it can be reopened and printed later
 * instead of keeping a flattened PDF.
 */
export function saveSheetProject(
  destination: string,
  settings: SheetSettings,
  copies: SheetCopy[],
): Promise<string> {
  return invoke<string>("save_sheet_project", {
    destination,
    settings,
    copies: projectCopies(copies),
  });
}

export async function openSheetProject(path: string): Promise<LoadedProject> {
  const payload = await invoke<LoadedProjectPayload>("open_sheet_project", { path });
  return hydrateProject(payload);
}

/** A sheet kept in the library database, so no file has to be managed. */
export interface SavedSheet {
  id: string;
  name: string;
  updatedAt: number;
  copies: number;
}

export function listSavedSheets(): Promise<SavedSheet[]> {
  return invoke<SavedSheet[]>("list_saved_sheets");
}

export function saveSavedSheet(
  id: string | null,
  name: string,
  settings: SheetSettings,
  copies: SheetCopy[],
): Promise<SavedSheet> {
  return invoke<SavedSheet>("save_saved_sheet", {
    id,
    name,
    payload: JSON.stringify({ version: 1, settings, copies: projectCopies(copies) }),
  });
}

export async function openSavedSheet(id: string): Promise<LoadedProject> {
  const payload = await invoke<string>("open_saved_sheet", { id });
  return hydrateProject(JSON.parse(payload) as LoadedProjectPayload);
}

export function renameSavedSheet(id: string, name: string): Promise<SavedSheet> {
  return invoke<SavedSheet>("rename_saved_sheet", { id, name });
}

export function deleteSavedSheet(id: string): Promise<number> {
  return invoke<number>("delete_saved_sheet", { id });
}

export interface NestCopyInput {
  id: string;
  path: string;
  widthMm: number;
  heightMm: number;
  /** Turned to by hand; the nester tries it first. */
  rotation: number;
}

export interface NestSettingsInput {
  paper: string;
  orientation: string;
  marginMm: number;
  gapMm: number;
  cutLines: boolean;
  outlineWidthMm: number;
  outlinePadMm: number;
  customWidthMm: number;
  customHeightMm: number;
}

export interface NestPlacementPayload {
  copyId: string;
  xMm: number;
  yMm: number;
  widthMm: number;
  heightMm: number;
  rotation: number;
  artXMm: number;
  artYMm: number;
}

export interface NestPayload {
  pageW: number;
  pageH: number;
  areaX: number;
  areaY: number;
  areaW: number;
  areaH: number;
  pages: NestPlacementPayload[][];
  unplaced: string[];
  inkMm2: number;
}

/** Fits every copy by its silhouette; the heavy work happens in Rust. */
export function nestSheet(
  copies: NestCopyInput[],
  settings: NestSettingsInput,
): Promise<NestPayload> {
  return invoke<NestPayload>("nest_sheet", { copies, settings });
}

/** Silhouette outlines in image-normalized coordinates (0..1, y down). */
export function stickerContour(
  path: string,
  widthMm: number,
  heightMm: number,
  paddingMm: number,
): Promise<[number, number][][]> {
  return invoke<[number, number][][]>("sticker_contour", {
    path,
    widthMm,
    heightMm,
    paddingMm,
  });
}

export async function pickImages(): Promise<string[]> {
  const picked = await open({
    multiple: true,
    title: "Importar imágenes",
    filters: [
      {
        name: "Imágenes",
        extensions: ["png", "svg", "jpg", "jpeg", "webp"],
      },
    ],
  });
  if (!picked) return [];
  return Array.isArray(picked) ? picked : [picked];
}
