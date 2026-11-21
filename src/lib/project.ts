import {
  MAX_SIZE_MM,
  MIN_SIZE_MM,
  clamp,
  type Rotation,
  type SheetCopy,
  type SheetSettings,
} from "./sheet";

/** Copy shape the backend stores inside `hoja.json`. */
export interface ProjectCopyInput {
  id: string;
  stickerId: string;
  name: string;
  ext: string;
  filePath: string;
  thumbPath: string;
  widthMm: number;
  heightMm: number;
  aspect: number;
  rotation: number;
}

export function projectCopies(copies: SheetCopy[]): ProjectCopyInput[] {
  return copies.map((copy) => ({
    id: copy.id,
    stickerId: copy.stickerId,
    name: copy.name,
    ext: copy.ext,
    filePath: copy.filePath || copy.thumbPath,
    thumbPath: copy.thumbPath,
    widthMm: copy.widthMm,
    heightMm: copy.heightMm,
    aspect: copy.aspect,
    rotation: copy.rotation,
  }));
}

export interface LoadedProjectPayload {
  /** Set when an old folder sheet was converted to a `.sns` file on open. */
  convertedTo?: string | null;
  settings: SheetSettings;
  copies: {
    id: string;
    stickerId: string;
    name: string;
    ext: string;
    filePath: string;
    thumbPath: string;
    widthMm: number;
    heightMm: number;
    aspect: number;
    rotation: number;
  }[];
}

export interface LoadedProject {
  convertedTo: string | null;
  settings: SheetSettings;
  copies: SheetCopy[];
}

export function quarterTurn(value: number): Rotation {
  if (!Number.isFinite(value)) return 0;
  const turns = ((Math.round(value) % 360) + 360) % 360;
  return ((Math.floor(turns / 90) * 90) % 360) as Rotation;
}

/**
 * Rebuilds app state from a saved sheet. Values are clamped to the ranges the
 * sheet UI supports, so an old or hand-edited project cannot break the layout.
 */
export function hydrateProject(payload: LoadedProjectPayload): LoadedProject {
  const raw = payload.settings ?? ({} as SheetSettings);
  const settings: SheetSettings = {
    paper: raw.paper === "letter" ? "letter" : "a4",
    orientation: raw.orientation === "landscape" ? "landscape" : "portrait",
    marginMm: clamp(Number(raw.marginMm), 0, 100),
    gapMm: clamp(Number(raw.gapMm), 0, 30),
    cutLines: raw.cutLines !== false,
    outlineWidthMm: clamp(Number(raw.outlineWidthMm), 0.05, 5),
    outlinePadMm: clamp(Number(raw.outlinePadMm), 0, 30),
  };

  const seen = new Set<string>();
  const copies: SheetCopy[] = [];
  for (const copy of payload.copies ?? []) {
    const filePath = copy.filePath || copy.thumbPath;
    if (!filePath) continue;
    const widthMm = clamp(Number(copy.widthMm), MIN_SIZE_MM, MAX_SIZE_MM);
    const heightMm = clamp(Number(copy.heightMm), MIN_SIZE_MM, MAX_SIZE_MM);
    const id = copy.id && !seen.has(copy.id) ? copy.id : `p${copies.length}-${copy.id || "copy"}`;
    seen.add(id);
    copies.push({
      id,
      stickerId: copy.stickerId || "",
      name: copy.name || "Sticker",
      ext: copy.ext || "png",
      filePath,
      thumbPath: copy.thumbPath || filePath,
      widthMm,
      heightMm,
      aspect:
        Number.isFinite(copy.aspect) && copy.aspect > 0 ? copy.aspect : widthMm / heightMm,
      rotation: quarterTurn(copy.rotation),
    });
  }

  return { convertedTo: payload.convertedTo ?? null, settings, copies };
}
