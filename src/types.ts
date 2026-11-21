export type StickerKind = "raster" | "vector";

export interface Sticker {
  id: string;
  name: string;
  ext: string;
  kind: StickerKind;
  widthPx: number;
  heightPx: number;
  widthMm: number;
  heightMm: number;
  dpi: number;
  hasAlpha: boolean;
  sizeBytes: number;
  createdAt: number;
  fileName: string;
  filePath: string;
  thumbPath: string;
  tags: string[];
  /** When the AI last named and tagged this sticker (0 = never). */
  aiAt: number;
}

export interface TagCount {
  tag: string;
  count: number;
}

export interface ImportFailure {
  path: string;
  message: string;
}

export interface ImportReport {
  importedIds: string[];
  errors: ImportFailure[];
}

export interface LibraryInfo {
  root: string;
  count: number;
  bytes: number;
}

export type SortKey = "recent" | "name" | "size";

export interface AiKeyInfo {
  id: string;
  name: string;
  /** Tail of the saved key, so it can be recognised without exposing it. */
  hint: string;
}

export interface AiStatus {
  /** In execution order: the first one is used first. */
  keys: AiKeyInfo[];
  model: string;
}

export interface AiKeyCheck {
  status: "ok" | "limited" | "invalid" | "error";
  message: string;
  /** Seconds Groq asked to wait, when it is limited. */
  wait: number;
  ms: number;
}

export interface AiFailure {
  id: string;
  message: string;
}

export interface AiReport {
  updated: string[];
  failures: AiFailure[];
}
