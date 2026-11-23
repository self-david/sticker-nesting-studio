import {
  clamp,
  pageDimensions,
  type SheetCopy,
  type SheetSettings,
} from "./sheet";
import { isTauri, nestSheet as nestInRust } from "./api";

/** One copy already placed on a page. */
export interface Placement {
  copy: SheetCopy;
  x: number;
  y: number;
  /** Box the copy occupies, cut line included. */
  widthMm: number;
  heightMm: number;
  /** Clockwise degrees: a multiple of 45. */
  rotation: number;
  /** Top-left of the artwork box inside that footprint, in millimetres. */
  artX: number;
  artY: number;
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
  /** Silhouette area actually stuck on the paper, in mm². */
  inkMm2: number;
}

/** Sheet with nothing on it, used before the first nesting finishes. */
export function emptyLayout(settings: SheetSettings): SheetLayout {
  const { w, h } = pageDimensions(settings);
  const margin = clamp(settings.marginMm, 0, Math.max(1, Math.min(w, h) / 2 - 1));
  return {
    pageW: w,
    pageH: h,
    areaX: margin,
    areaY: margin,
    areaW: Math.max(0, w - margin * 2),
    areaH: Math.max(0, h - margin * 2),
    // One blank page, so the sheet always has something to show and the page
    // index never has to cope with a layout without pages.
    pages: [[]],
    unplaced: [],
    inkMm2: 0,
  };
}

/**
 * Two readings of the same sheet, because they answer different questions:
 * `ink` is the silhouette really printed; `box` is the artwork's own rectangle
 * once turned, transparent corners and cut margin aside, which is exactly what
 * the old rectangle layout used to report.
 */
export function layoutStats(layout: SheetLayout): {
  placed: number;
  ink: number;
  box: number;
} {
  let placed = 0;
  let boxMm2 = 0;
  for (const page of layout.pages) {
    placed += page.length;
    for (const p of page) {
      // Bounding box of the artwork once turned, in millimetres.
      const radians = (p.rotation * Math.PI) / 180;
      const sin = Math.abs(Math.sin(radians));
      const cos = Math.abs(Math.cos(radians));
      const { widthMm: w, heightMm: h } = p.copy;
      boxMm2 += (w * cos + h * sin) * (w * sin + h * cos);
    }
  }
  const available = layout.areaW * layout.areaH * layout.pages.length;
  const share = (mm2: number) => (available > 0 ? mm2 / available : 0);
  return { placed, ink: share(layout.inkMm2), box: share(boxMm2) };
}

/**
 * Fits the sheet so each sticker's silhouette decides where it goes. The
 * tracing and the packing happen in Rust; here the result is joined back with
 * the copies the UI already holds.
 */
export async function nestSheet(
  copies: SheetCopy[],
  settings: SheetSettings,
): Promise<SheetLayout> {
  if (!isTauri) return { ...emptyLayout(settings), unplaced: [...copies] };

  const result = await nestInRust(
    copies.map((copy) => ({
      id: copy.id,
      // The preview has the same shape as the artwork and decodes in a moment:
      // the mask only needs the silhouette, not print resolution.
      path: copy.thumbPath || copy.filePath,
      widthMm: copy.widthMm,
      heightMm: copy.heightMm,
      rotation: copy.rotation,
    })),
    {
      paper: settings.paper,
      orientation: settings.orientation,
      marginMm: settings.marginMm,
      gapMm: settings.gapMm,
      cutLines: settings.cutLines,
      outlineWidthMm: settings.outlineWidthMm,
      outlinePadMm: settings.outlinePadMm,
      customWidthMm: settings.customWidthMm,
      customHeightMm: settings.customHeightMm,
    },
  );

  const byId = new Map(copies.map((copy) => [copy.id, copy]));
  const pages: Placement[][] = [];
  for (const page of result.pages) {
    const placed: Placement[] = [];
    for (const item of page) {
      const copy = byId.get(item.copyId);
      if (!copy) continue;
      placed.push({
        copy,
        x: item.xMm,
        y: item.yMm,
        widthMm: item.widthMm,
        heightMm: item.heightMm,
        rotation: item.rotation,
        artX: item.artXMm,
        artY: item.artYMm,
      });
    }
    if (placed.length > 0) pages.push(placed);
  }

  return {
    pageW: result.pageW,
    pageH: result.pageH,
    areaX: result.areaX,
    areaY: result.areaY,
    areaW: result.areaW,
    areaH: result.areaH,
    pages,
    unplaced: result.unplaced
      .map((id) => byId.get(id))
      .filter((copy): copy is SheetCopy => Boolean(copy)),
    inkMm2: result.inkMm2,
  };
}
