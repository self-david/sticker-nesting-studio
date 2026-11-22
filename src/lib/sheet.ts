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
  /** Page width used when `paper` is "custom", in millimetres. */
  customWidthMm: number;
  /** Page height used when `paper` is "custom", in millimetres. */
  customHeightMm: number;
}

export const DEFAULT_SETTINGS: SheetSettings = {
  paper: "a4",
  orientation: "portrait",
  marginMm: 8,
  gapMm: 3,
  cutLines: true,
  outlineWidthMm: 0.3,
  outlinePadMm: 1.5,
  customWidthMm: 210,
  customHeightMm: 297,
};

export const MIN_SIZE_MM = 1;
export const MAX_SIZE_MM = 2000;
/** Most copies of one sticker the sheet accepts in one go. */
export const MAX_COPIES = 200;

/** Quarter turns applied to a single copy. */
export type Rotation = 0 | 90 | 180 | 270;

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
  /** Quarter turns turned on the sheet, so tighter nesting is possible. */
  rotation: Rotation;
}

export interface Placement {
  copy: SheetCopy;
  x: number;
  y: number;
  widthMm: number;
  heightMm: number;
  /** Orientation the copy is printed at (its footprint is already rotated). */
  rotation: Rotation;
}

export interface SheetLayout {
  pageW: number;
  pageH: number;
  areaX: number;
  areaY: number;
  areaW: number;
  areaH: number;
  pages: Placement[][];
  unplaced: SheetCopy[];
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

export function copyFromSticker(sticker: Sticker): SheetCopy {
  return {
    id: newId(),
    stickerId: sticker.id,
    name: sticker.name,
    ext: sticker.ext,
    filePath: sticker.filePath || sticker.thumbPath,
    thumbPath: sticker.thumbPath,
    widthMm: sticker.widthMm,
    heightMm: sticker.heightMm,
    aspect: sticker.heightMm > 0 ? sticker.widthMm / sticker.heightMm : 1,
    rotation: 0,
  };
}

/**
 * Copies of one sticker, ready for the sheet. The quantity is rounded and
 * clamped, so a typed or dragged value can never add nothing or thousands.
 */
export function copiesFromSticker(sticker: Sticker, quantity: number): SheetCopy[] {
  const count = clamp(Math.round(quantity) || 1, 1, MAX_COPIES);
  return Array.from({ length: count }, () => copyFromSticker(sticker));
}

/** Size the copy occupies on the sheet once its rotation is applied. */
export function footprint(copy: SheetCopy): { w: number; h: number } {
  return copy.rotation === 90 || copy.rotation === 270
    ? { w: copy.heightMm, h: copy.widthMm }
    : { w: copy.widthMm, h: copy.heightMm };
}

export function isQuarterTurn(rotation: Rotation): boolean {
  return rotation === 90 || rotation === 270;
}

/** Next quarter turn, used by the rotate button on each copy. */
export function rotateCopy(copy: SheetCopy): SheetCopy {
  return { ...copy, rotation: ((copy.rotation + 90) % 360) as Rotation };
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

/** Largest margin that still leaves a printable area. */
export function maxMarginMm(pageW: number, pageH: number): number {
  return Math.max(1, Math.floor(Math.min(pageW, pageH) / 2 - 1));
}

/**
 * Places copies bottom-left inside the printable area: each piece goes to the
 * lowest, then leftmost free spot, so pockets left by tall stickers are used.
 * A piece that finds no hole is tried a quarter turn before a new page opens;
 * the rotation it ends up with is stored on the placement. Pages overflow
 * automatically and oversized copies are reported, never shrunk.
 */
export function layoutSheet(copies: SheetCopy[], settings: SheetSettings): SheetLayout {
  const { w: pageW, h: pageH } = pageDimensions(settings);
  const margin = clamp(settings.marginMm, 0, maxMarginMm(pageW, pageH));
  const gap = clamp(settings.gapMm, 0, 30);
  const areaX = margin;
  const areaY = margin;
  const areaW = Math.max(0, pageW - margin * 2);
  const areaH = Math.max(0, pageH - margin * 2);

  const pages: Placement[][] = [[]];
  const unplaced: SheetCopy[] = [];
  const layout: SheetLayout = { pageW, pageH, areaX, areaY, areaW, areaH, pages, unplaced };

  // The cut line reaches past the copy box, so the layout reserves that room:
  // otherwise an outline lands on the margin or on the outline next to it.
  const room = settings.cutLines
    ? Math.max(0, settings.outlinePadMm) + Math.max(0, settings.outlineWidthMm) / 2
    : 0;
  const packX = areaX + room;
  const packY = areaY + room;
  const packW = Math.max(0, areaW - room * 2);
  const packH = Math.max(0, areaH - room * 2);
  // Two outlines need twice the room between the boxes they belong to.
  const spacing = gap + room * 2;

  if (areaW <= 0 || areaH <= 0 || packW <= 0 || packH <= 0) {
    layout.unplaced = [...copies];
    return layout;
  }

  const eps = 1e-9;
  const right = packX + packW;
  const bottom = packY + packH;

  const fits = (spot: { x: number; y: number }, w: number, h: number, placed: Placement[]) => {
    if (spot.x + w > right + eps || spot.y + h > bottom + eps) return false;
    return placed.every(
      (r) =>
        spot.x + w + spacing <= r.x + eps ||
        r.x + r.widthMm + spacing <= spot.x + eps ||
        spot.y + h + spacing <= r.y + eps ||
        r.y + r.heightMm + spacing <= spot.y + eps,
    );
  };

  const findSpot = (placed: Placement[], w: number, h: number) => {
    const xs = new Set<number>([packX]);
    const ys = new Set<number>([packY]);
    for (const r of placed) {
      xs.add(r.x + r.widthMm + spacing);
      ys.add(r.y + r.heightMm + spacing);
    }
    const sortedX = [...xs].sort((a, b) => a - b);
    const sortedY = [...ys].sort((a, b) => a - b);
    for (const y of sortedY) {
      for (const x of sortedX) {
        const spot = { x, y };
        if (fits(spot, w, h, placed)) return spot;
      }
    }
    return null;
  };

  // Largest footprint first, then tallest, so pockets are filled predictably.
  const ordered = [...copies].sort((a, b) => {
    const fa = footprint(a);
    const fb = footprint(b);
    return (
      fb.w * fb.h - fa.w * fa.h ||
      fb.h - fa.h ||
      a.name.localeCompare(b.name)
    );
  });

  // Base orientation first, then the same piece a quarter turn later; only
  // the ones that fit the printable area are worth trying.
  const fitsArea = (w: number, h: number) =>
    w > 0 && h > 0 && w <= packW + eps && h <= packH + eps;

  const orientations = (copy: SheetCopy) => {
    const { w, h } = footprint(copy);
    const options = [
      { w, h, rotation: copy.rotation },
      { w: h, h: w, rotation: ((copy.rotation + 90) % 360) as Rotation },
    ];
    return (Math.abs(w - h) > eps ? options : options.slice(0, 1)).filter((o) =>
      fitsArea(o.w, o.h),
    );
  };

  const place = (
    copy: SheetCopy,
    options: { w: number; h: number; rotation: Rotation }[],
    onPage: Placement[],
  ): Placement | null => {
    for (const { w, h, rotation } of options) {
      const spot = findSpot(onPage, w, h);
      if (spot) {
        return { copy, x: spot.x, y: spot.y, widthMm: w, heightMm: h, rotation };
      }
    }
    return null;
  };

  let placed: Placement[] = [];
  for (const copy of ordered) {
    const options = orientations(copy);
    if (options.length === 0) {
      unplaced.push(copy);
      continue;
    }
    // The current page, in either orientation, before a new page is opened.
    let placement = place(copy, options, placed);
    if (!placement) {
      pages.push([]);
      placed = [];
      placement = place(copy, options, placed);
    }
    if (!placement) {
      unplaced.push(copy);
      continue;
    }
    pages[pages.length - 1].push(placement);
    placed.push(placement);
  }

  return layout;
}

export function layoutStats(layout: SheetLayout): { placed: number; fill: number } {
  let placed = 0;
  let used = 0;
  for (const page of layout.pages) {
    for (const p of page) {
      placed += 1;
      used += p.widthMm * p.heightMm;
    }
  }
  const available = layout.areaW * layout.areaH * layout.pages.length;
  return { placed, fill: available > 0 ? used / available : 0 };
}
