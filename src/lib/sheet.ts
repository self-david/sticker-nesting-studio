import type { Sticker } from "../types";
import { round1 } from "./format";

export type PaperId = "a4" | "letter" | "custom";
export type Orientation = "portrait" | "landscape";

export const PAPER_MM: Record<"a4" | "letter", { w: number; h: number }> = {
  a4: { w: 210, h: 297 },
  letter: { w: 215.9, h: 279.4 },
};

/** Bounds for a page the user measures out themselves, in millimetres. */
export const MIN_PAPER_MM = 20;
export const MAX_PAPER_MM = 2000;

export interface SheetSettings {
  paper: PaperId;
  orientation: Orientation;
  marginMm: number;
  gapMm: number;
  cutLines: boolean;
  outlineWidthMm: number;
  outlinePadMm: number;
  /** Colour of the border drawn around each sticker, and of its cut line. */
  outlineColor: string;
  /** Fill inside that border; "" leaves the artwork's transparency alone. */
  stickerBg: string;
  /** Page width used when `paper` is "custom", in millimetres. */
  customWidthMm: number;
  /** Page height used when `paper` is "custom", in millimetres. */
  customHeightMm: number;
  /**
   * Area given to every new copy, in cm² (width × height), so the stickers on
   * a sheet come out about the same size. 0 keeps each sticker's real size.
   */
  areaCm2: number;
}

export const DEFAULT_SETTINGS: SheetSettings = {
  paper: "a4",
  orientation: "portrait",
  marginMm: 8,
  gapMm: 3,
  cutLines: true,
  outlineWidthMm: 0.3,
  outlinePadMm: 1.5,
  outlineColor: "#000000",
  stickerBg: "",
  customWidthMm: 210,
  customHeightMm: 297,
  areaCm2: 0,
};

/** Border colour a sheet starts with. */
export const DEFAULT_BORDER_COLOR = "#000000";

/** `#rgb` or `#rrggbb` as `#rrggbb`; anything else falls back. */
export function normalizeColor(raw: unknown, fallback: string): string {
  if (typeof raw !== "string") return fallback;
  const value = raw.trim().toLowerCase();
  if (/^#[0-9a-f]{6}$/.test(value)) return value;
  if (/^#[0-9a-f]{3}$/.test(value)) {
    return `#${value[1]}${value[1]}${value[2]}${value[2]}${value[3]}${value[3]}`;
  }
  return fallback;
}

/** Largest area the size field accepts, in cm². */
export const MAX_AREA_CM2 = 2000;

export const MIN_SIZE_MM = 1;
export const MAX_SIZE_MM = 2000;
/** Most copies of one sticker the sheet accepts in one go. */
export const MAX_COPIES = 200;

/** Clockwise turn of a single copy, in degrees (multiples of 45). */
export type Rotation = number;

/** One physical copy on the sheet. Each copy keeps its own size. */
export interface SheetCopy {
  id: string;
  stickerId: string;
  name: string;
  ext: string;
  filePath: string;
  thumbPath: string;
  widthMm: number;
  heightMm: number;
  /** Original width / height, kept so every size edit preserves the shape. */
  aspect: number;
  /**
   * Turn of the copy on the sheet, in degrees. The nester tries it first and
   * falls back to the other 45° steps when that angle does not fit.
   */
  rotation: Rotation;
}

export function clamp(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, value));
}

export function newId(): string {
  const cryptoApi = globalThis.crypto;
  if (cryptoApi && typeof cryptoApi.randomUUID === "function") {
    return cryptoApi.randomUUID();
  }
  return `c${Date.now().toString(36)}${Math.random().toString(36).slice(2, 9)}`;
}

/**
 * Size that gives the sticker `areaCm2` square centimetres without changing
 * its shape: width = √(area × aspect), height = width / aspect. Null means
 * "keep the real size".
 */
export function sizeFromArea(
  aspect: number,
  areaCm2: number,
): { widthMm: number; heightMm: number } | null {
  if (!(areaCm2 > 0)) return null;
  const ratio = aspect > 0 ? aspect : 1;
  // 1 cm² is 100 mm².
  const width = Math.sqrt(areaCm2 * 100 * ratio);
  if (!Number.isFinite(width) || width <= 0) return null;
  return {
    widthMm: clamp(round1(width), MIN_SIZE_MM, MAX_SIZE_MM),
    heightMm: clamp(round1(width / ratio), MIN_SIZE_MM, MAX_SIZE_MM),
  };
}

/**
 * Gives every copy on the sheet the same area, keeping each sticker's shape.
 * Used to even out sizes after the sheet already has pieces on it.
 */
export function resizeCopiesToArea(copies: SheetCopy[], areaCm2: number): SheetCopy[] {
  return copies.map((copy) => {
    const aspect = copy.aspect > 0 ? copy.aspect : copy.widthMm / copy.heightMm;
    const sized = sizeFromArea(aspect, areaCm2);
    return sized ? { ...copy, ...sized } : copy;
  });
}

export function copyFromSticker(sticker: Sticker, areaCm2 = 0): SheetCopy {
  const aspect = sticker.heightMm > 0 ? sticker.widthMm / sticker.heightMm : 1;
  const sized = sizeFromArea(aspect, areaCm2);
  return {
    id: newId(),
    stickerId: sticker.id,
    name: sticker.name,
    ext: sticker.ext,
    filePath: sticker.filePath || sticker.thumbPath,
    thumbPath: sticker.thumbPath,
    widthMm: sized?.widthMm ?? sticker.widthMm,
    heightMm: sized?.heightMm ?? sticker.heightMm,
    aspect,
    rotation: 0,
  };
}

/**
 * Copies of one sticker, ready for the sheet. The quantity is rounded and
 * clamped, so a typed or dragged value can never add nothing or thousands.
 */
export function copiesFromSticker(
  sticker: Sticker,
  quantity: number,
  areaCm2 = 0,
): SheetCopy[] {
  const count = clamp(Math.round(quantity) || 1, 1, MAX_COPIES);
  return Array.from({ length: count }, () => copyFromSticker(sticker, areaCm2));
}

/** Next turn, used by the rotate button on each copy. */
export function rotateCopy(copy: SheetCopy): SheetCopy {
  return { ...copy, rotation: (copy.rotation + 45) % 360 };
}

/**
 * Resizes a copy while keeping the sticker's proportions: editing one
 * dimension derives the other, and both stay inside the printable limits.
 */
export function resizeCopy(
  copy: SheetCopy,
  patch: { widthMm?: number; heightMm?: number },
): SheetCopy {
  const aspect =
    copy.aspect > 0 ? copy.aspect : copy.heightMm > 0 ? copy.widthMm / copy.heightMm : 1;
  if (patch.widthMm != null) {
    const widthMm = clamp(patch.widthMm, MIN_SIZE_MM, MAX_SIZE_MM);
    return {
      ...copy,
      widthMm,
      heightMm: clamp(round1(widthMm / aspect), MIN_SIZE_MM, MAX_SIZE_MM),
    };
  }
  if (patch.heightMm != null) {
    const heightMm = clamp(patch.heightMm, MIN_SIZE_MM, MAX_SIZE_MM);
    return {
      ...copy,
      heightMm,
      widthMm: clamp(round1(heightMm * aspect), MIN_SIZE_MM, MAX_SIZE_MM),
    };
  }
  return copy;
}

export function paperSize(settings: SheetSettings): { w: number; h: number } {
  if (settings.paper === "custom") {
    return {
      w: clamp(settings.customWidthMm, MIN_PAPER_MM, MAX_PAPER_MM),
      h: clamp(settings.customHeightMm, MIN_PAPER_MM, MAX_PAPER_MM),
    };
  }
  return PAPER_MM[settings.paper];
}

export function pageDimensions(settings: SheetSettings): { w: number; h: number } {
  const base = paperSize(settings);
  return settings.orientation === "landscape" ? { w: base.h, h: base.w } : { w: base.w, h: base.h };
}

/**
 * File name a sheet is exported under: its own name when it has one, so the
 * PDF and the `.sns` next to it read the same, and a neutral name otherwise.
 */
export function sheetFileName(
  name: string,
  fallback: string,
  extension: string,
): string {
  // Characters Windows rejects in a file name, and trailing dots or spaces.
  const clean = name
    .replace(/[\\/:*?"<>|]/g, "")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/[. ]+$/, "")
    .slice(0, 80);
  return `${clean || fallback}.${extension}`;
}

/** Largest margin that still leaves a printable area. */
export function maxMarginMm(pageW: number, pageH: number): number {
  return Math.max(1, Math.floor(Math.min(pageW, pageH) / 2 - 1));
}
