import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  AiKeyCheck,
  AiReport,
  AiStatus,
  ImportReport,
  LibraryInfo,
  SortKey,
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

export function importPaths(paths: string[]): Promise<ImportReport> {
  return invoke<ImportReport>("import_paths", { paths });
}

export function listStickers(query: string, tags: string[], sort: SortKey): Promise<Sticker[]> {
  return invoke<Sticker[]>("list_stickers", { query, tags, sort });
}

export function listTags(): Promise<TagCount[]> {
  return invoke<TagCount[]>("list_tags");
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

export interface PdfExportItem {
  path: string;
  xMm: number;
  yMm: number;
  /** Footprint on the sheet (already rotated). */
  widthMm: number;
  heightMm: number;
  /** Degrees clockwise: 0, 90, 180 or 270. */
  rotation: number;
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
