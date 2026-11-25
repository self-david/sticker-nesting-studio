import {
  DEFAULT_BORDER_COLOR,
  MAX_AREA_CM2,
  MAX_PAPER_MM,
  MAX_SIZE_MM,
  MIN_PAPER_MM,
  MIN_SIZE_MM,
  clamp,
  normalizeColor,
  type PaperId,
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

/** Any turn the nester uses (multiples of 45) survives a save and reopen. */
export function normalizeAngle(value: number): Rotation {
  if (!Number.isFinite(value)) return 0;
  const turns = ((Math.round(value) % 360) + 360) % 360;
  return (Math.round(turns / 45) * 45) % 360;
}

/**
 * Rebuilds app state from a saved sheet. Values are clamped to the ranges the
 * sheet UI supports, so an old or hand-edited project cannot break the layout.
 */
export function hydrateProject(payload: LoadedProjectPayload): LoadedProject {
  const raw = payload.settings ?? ({} as SheetSettings);
  const paper: PaperId =
    raw.paper === "letter" || raw.paper === "custom" ? raw.paper : "a4";
  const settings: SheetSettings = {
    paper,
    orientation: raw.orientation === "landscape" ? "landscape" : "portrait",
    marginMm: clamp(Number(raw.marginMm), 0, 100),
    gapMm: clamp(Number(raw.gapMm), 0, 30),
    cutLines: raw.cutLines !== false,
    outlineWidthMm: clamp(Number(raw.outlineWidthMm), 0.05, 5),
    outlinePadMm: clamp(Number(raw.outlinePadMm), 0, 30),
    outlineColor: normalizeColor(raw.outlineColor, DEFAULT_BORDER_COLOR),
    stickerBg: raw.stickerBg ? normalizeColor(raw.stickerBg, "") : "",
    customWidthMm: clamp(Number(raw.customWidthMm) || 210, MIN_PAPER_MM, MAX_PAPER_MM),
    customHeightMm: clamp(Number(raw.customHeightMm) || 297, MIN_PAPER_MM, MAX_PAPER_MM),
    areaCm2: clamp(Number(raw.areaCm2) || 0, 0, MAX_AREA_CM2),
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
      rotation: normalizeAngle(copy.rotation),
    });
  }

  return { convertedTo: payload.convertedTo ?? null, settings, copies };
}
