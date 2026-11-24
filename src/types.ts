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
  /** The work or franchise, in one canonical spelling ("" = not classified yet). */
  serie: string;
  /** One of the closed family list ("" = not classified yet). */
  tipo: string;
  /** When the AI last named and tagged this sticker (0 = never). */
  aiAt: number;
  /** When the text classifier filled serie and tipo (0 = never). */
  classifiedAt: number;
  /** Words printed on the artwork, as the AI read them ("" = none). */
  texto: string;
}

export interface TagCount {
  tag: string;
  count: number;
}

/** One value of the series or type facet; "" is the «no series» bucket. */
export interface FacetCount {
  value: string;
  count: number;
}

export interface ImportFailure {
  path: string;
  message: string;
}

export interface ImportReport {
  importedIds: string[];
  errors: ImportFailure[];
  /** Paths the library already had, skipped unless the user insists. */
  duplicates: string[];
  cancelled: boolean;
}

export interface ImportProgress {
  done: number;
  total: number;
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
  /** The vision model: names and tags from the artwork. */
  model: string;
  /** The text model used to classify series and types. */
  textModel: string;
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

/** One canonical name (a tag, a series) and the spellings to fold into it. */
export interface MergeGroup {
  canonico: string;
  variantes: string[];
}

export interface AiReport {
  updated: string[];
  failures: AiFailure[];
}
