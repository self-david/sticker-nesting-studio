import { describe, expect, test } from "bun:test";
import {
  RECTANGLE_CONTOUR,
  cachedContours,
  clearContourCache,
  rememberContour,
  rememberFailure,
  retryFailedContours,
} from "./contour";
import {
  DEFAULT_SETTINGS,
  MAX_COPIES,
  MAX_SIZE_MM,
  MIN_SIZE_MM,
  copiesFromSticker,
  resizeCopiesToArea,
  resizeCopy,
  rotateCopy,
  pageDimensions,
  sheetFileName,
  sizeFromArea,
  type Rotation,
  type SheetCopy,
} from "./sheet";
import type { Sticker } from "../types";

describe("size from area", () => {
  test("keeps the shape and gives the asked area", () => {
    // 20 cm² on a 4:3 sticker: 16.3 × 12.2 mm… squared is 20 cm².
    const sized = sizeFromArea(4 / 3, 20)!;
    expect(sized).not.toBeNull();
    expect(sized.widthMm / sized.heightMm).toBeCloseTo(4 / 3, 1);
    expect((sized.widthMm * sized.heightMm) / 100).toBeCloseTo(20, 0);
  });

  test("zero means keeping the sticker's real size", () => {
    expect(sizeFromArea(1, 0)).toBeNull();
  });

  test("the copies already on the sheet take the area and keep their shape", () => {
    const even = resizeCopiesToArea([copy("a", 40, 20), copy("b", 12, 36, 1 / 3)], 30);
    expect(even[0].widthMm * even[0].heightMm).toBeCloseTo(3000, -1);
    expect(even[1].widthMm * even[1].heightMm).toBeCloseTo(3000, -1);
    expect(even[1].widthMm / even[1].heightMm).toBeCloseTo(1 / 3, 1);
    expect(even[0].id).toBe("a");
  });
});

describe("custom paper", () => {
  const custom = {
    ...DEFAULT_SETTINGS,
    paper: "custom" as const,
    customWidthMm: 330,
    customHeightMm: 120,
  };

  test("measures the page in millimetres and turns with the orientation", () => {
    expect(pageDimensions(custom)).toEqual({ w: 330, h: 120 });
    expect(pageDimensions({ ...custom, orientation: "landscape" })).toEqual({ w: 120, h: 330 });
  });
});

const copy = (
  id: string,
  widthMm: number,
  heightMm: number,
  aspect = widthMm / heightMm,
  rotation: Rotation = 0,
): SheetCopy => ({
  id,
  stickerId: `s-${id}`,
  name: id,
  ext: "png",
  filePath: `C:/st/${id}.png`,
  thumbPath: `C:/st/${id}.png`,
  widthMm,
  heightMm,
  aspect,
  rotation,
});

const sticker = (widthMm = 40, heightMm = 20): Sticker => ({
  id: "s1",
  name: "Corona",
  ext: "png",
  kind: "raster",
  widthPx: 400,
  heightPx: 200,
  widthMm,
  heightMm,
  dpi: 300,
  hasAlpha: true,
  sizeBytes: 1200,
  createdAt: 1,
  fileName: "corona.png",
  filePath: "C:/lib/corona.png",
  thumbPath: "C:/lib/thumbs/corona.png",
  tags: ["realeza"],
  serie: "",
  tipo: "",
  aiAt: 0,
  classifiedAt: 0,
  texto: "",
});

describe("copiesFromSticker", () => {
  test("adds as many copies as asked, each one independent", () => {
    const copies = copiesFromSticker(sticker(), 3);
    expect(copies).toHaveLength(3);
    expect(new Set(copies.map((c) => c.id)).size).toBe(3);
    expect(copies[0].widthMm).toBe(40);
    expect(copies[0].aspect).toBe(2);
    expect(copies[0].filePath).toBe("C:/lib/corona.png");
  });

  test("clamps the quantity to what the sheet accepts", () => {
    expect(copiesFromSticker(sticker(), 0)).toHaveLength(1);
    expect(copiesFromSticker(sticker(), -8)).toHaveLength(1);
    expect(copiesFromSticker(sticker(), 2.4)).toHaveLength(2);
    expect(copiesFromSticker(sticker(), 10_000)).toHaveLength(MAX_COPIES);
    expect(copiesFromSticker(sticker(), Number.NaN)).toHaveLength(1);
  });
});

describe("export file name", () => {
  test("takes the sheet's name when it has one", () => {
    expect(sheetFileName("todos los stickers", "hoja-stickers", "pdf")).toBe(
      "todos los stickers.pdf",
    );
  });

  test("falls back to the neutral name for an unnamed sheet", () => {
    expect(sheetFileName("", "hoja-stickers", "pdf")).toBe("hoja-stickers.pdf");
    expect(sheetFileName("   ", "hoja-stickers", "sns")).toBe("hoja-stickers.sns");
  });

  test("drops what a file name cannot hold", () => {
    expect(sheetFileName("hoja/2: stickers?", "hoja", "pdf")).toBe("hoja2 stickers.pdf");
    expect(sheetFileName("fin.", "hoja", "pdf")).toBe("fin.pdf");
  });
});

describe("rotation", () => {
  test("turns in 45° steps and comes back round", () => {
    let current = copy("a", 80, 40);
    const seen: number[] = [];
    for (let i = 0; i < 8; i += 1) {
      current = rotateCopy(current);
      seen.push(current.rotation);
    }
    expect(seen).toEqual([45, 90, 135, 180, 225, 270, 315, 0]);
  });

  test("resizing keeps the rotation", () => {
    const next = resizeCopy(copy("a", 80, 40, undefined, 90), { widthMm: 60 });
    expect(next.rotation).toBe(90);
    expect(next.heightMm).toBe(30);
  });
});

describe("contour cache", () => {
  test("remembers successful traces across visits", () => {
    clearContourCache();
    const shape: [number, number][][] = [
      [
        [0, 0],
        [1, 0],
        [0, 1],
      ],
    ];
    rememberContour("a", shape);
    expect(cachedContours().a).toBe(shape);
  });

  test("a failure shows a rectangle and is retried next visit", () => {
    clearContourCache();
    rememberFailure("b");
    expect(cachedContours().b).toBe(RECTANGLE_CONTOUR);

    expect(retryFailedContours()).toBe(1);
    expect(cachedContours().b).toBeUndefined();
    expect(retryFailedContours()).toBe(0);
  });

  test("retrying failures keeps the good contours", () => {
    clearContourCache();
    const shape: [number, number][][] = [
      [
        [0, 0],
        [1, 1],
      ],
    ];
    rememberContour("ok", shape);
    rememberFailure("bad");
    retryFailedContours();
    expect(cachedContours().ok).toBe(shape);
    expect(cachedContours().bad).toBeUndefined();
  });

  test("a later success clears the failure mark", () => {
    clearContourCache();
    rememberFailure("c");
    rememberContour("c", [
      [
        [0, 0],
        [1, 0],
        [1, 1],
      ],
    ]);
    retryFailedContours();
    expect(cachedContours().c).toBeDefined();
  });
});

describe("resizeCopy", () => {
  test("derives the height from the width", () => {
    const next = resizeCopy(copy("a", 100, 50), { widthMm: 60 });
    expect(next.widthMm).toBe(60);
    expect(next.heightMm).toBe(30);
  });

  test("derives the width from the height", () => {
    const next = resizeCopy(copy("a", 100, 50), { heightMm: 25 });
    expect(next.heightMm).toBe(25);
    expect(next.widthMm).toBe(50);
  });

  test("keeps proportions of non-square stickers", () => {
    const next = resizeCopy(copy("a", 70, 40), { widthMm: 35 });
    expect(next.heightMm).toBe(20);
    expect(next.widthMm / next.heightMm).toBeCloseTo(70 / 40, 5);
  });

  test("clamps both dimensions to the printable limits", () => {
    const tiny = resizeCopy(copy("a", 100, 50), { widthMm: 0.2 });
    expect(tiny.widthMm).toBe(MIN_SIZE_MM);
    expect(tiny.heightMm).toBeGreaterThanOrEqual(MIN_SIZE_MM);

    const huge = resizeCopy(copy("a", 100, 50), { widthMm: 99999 });
    expect(huge.widthMm).toBe(MAX_SIZE_MM);
    expect(huge.heightMm).toBe(MAX_SIZE_MM / 2);
  });

  test("leaves the copy untouched without a size", () => {
    const original = copy("a", 100, 50);
    expect(resizeCopy(original, {})).toBe(original);
  });
});
