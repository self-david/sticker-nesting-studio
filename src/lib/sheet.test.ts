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
  footprint,
  resizeCopy,
  rotateCopy,
  layoutSheet,
  pageDimensions,
  type Rotation,
  type SheetCopy,
} from "./sheet";
import type { Sticker } from "../types";

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

  test("lays out copies inside the custom page", () => {
    const layout = layoutSheet([copy("a", 40, 20), copy("b", 40, 20)], custom);
    expect(layout.pageW).toBe(330);
    expect(layout.pageH).toBe(120);
    expect(layout.pages[0]).toHaveLength(2);
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

// Plain geometry, without the room reserved for cut lines.
const settings = { ...DEFAULT_SETTINGS, marginMm: 10, gapMm: 4, cutLines: false };

const placements = (copies: SheetCopy[]) => {
  const layout = layoutSheet(copies, settings);
  return {
    layout,
    byId: new Map(layout.pages.flat().map((p) => [p.copy.id, p])),
  };
};

describe("layoutSheet turns a copy to fit", () => {
  test("tries the quarter turn before starting a new page", () => {
    // The first piece fills the width, leaving a 73 mm strip under it: the
    // tall piece fits there only when it is turned.
    const { layout, byId } = placements([copy("big", 190, 200), copy("strip", 60, 150)]);

    expect(layout.pages).toHaveLength(1);
    const placement = byId.get("strip")!;
    expect(placement.rotation).toBe(90);
    expect(placement.widthMm).toBe(150);
    expect(placement.heightMm).toBe(60);
    expect(placement.y).toBeGreaterThanOrEqual(10 + 200 + 4);
  });

  test("keeps the copy's own orientation when it fits", () => {
    const { layout, byId } = placements([copy("big", 190, 200), copy("wide", 100, 50)]);

    expect(layout.pages).toHaveLength(1);
    expect(byId.get("wide")!.rotation).toBe(0);
    expect(byId.get("wide")!.widthMm).toBe(100);
  });

  test("turns a copy the user already rotated one more quarter", () => {
    // Upright it is 100 x 40 turned by the user, so its footprint is 40 x 100;
    // the strip only takes it at 100 x 40, which is one more quarter turn.
    const { byId } = placements([copy("big", 190, 200), copy("turned", 100, 40, 2.5, 90)]);

    expect(byId.get("turned")!.rotation).toBe(180);
    expect(byId.get("turned")!.widthMm).toBe(100);
    expect(byId.get("turned")!.heightMm).toBe(40);
  });

  test("places a copy that is too wide turned on its own page", () => {
    const { layout, byId } = placements([copy("tooWide", 220, 150)]);

    expect(layout.unplaced).toHaveLength(0);
    expect(layout.pages).toHaveLength(1);
    const placement = byId.get("tooWide")!;
    expect(placement.rotation).toBe(90);
    expect(placement.widthMm).toBe(150);
    expect(placement.heightMm).toBe(220);
  });

  test("reports a copy that fits in neither orientation, without an empty page", () => {
    const { layout } = placements([copy("huge", 300, 300)]);

    expect(layout.pages).toHaveLength(1);
    expect(layout.pages[0]).toHaveLength(0);
    expect(layout.unplaced.map((c) => c.id)).toEqual(["huge"]);
  });
});

describe("layoutSheet", () => {
  test("fills the pocket a row packer would leave empty", () => {
    const { byId } = placements([
      copy("tall", 110, 110),
      copy("wide", 90, 30),
      copy("star", 50, 50),
      copy("small", 40, 40),
    ]);

    const star = byId.get("star")!;
    const small = byId.get("small")!;
    // Both sit to the right of the tall sticker, at the top of the area.
    expect(star.x).toBe(124);
    expect(star.y).toBe(10);
    expect(small.x).toBe(124);
    expect(small.y).toBe(64);
  });

  test("keeps every copy inside the area and apart by the gap", () => {
    const copies = [
      copy("a", 100, 80),
      copy("b", 70, 120),
      copy("c", 60, 40),
      copy("d", 55, 55),
      copy("e", 90, 35),
      copy("f", 40, 90),
    ];
    const { layout } = placements(copies);
    const all = layout.pages.flat();
    expect(all).toHaveLength(copies.length);

    for (const p of all) {
      expect(p.x).toBeGreaterThanOrEqual(layout.areaX);
      expect(p.y).toBeGreaterThanOrEqual(layout.areaY);
      expect(p.x + p.widthMm).toBeLessThanOrEqual(layout.areaX + layout.areaW);
      expect(p.y + p.heightMm).toBeLessThanOrEqual(layout.areaY + layout.areaH);
    }

    for (const page of layout.pages) {
      for (let i = 0; i < page.length; i += 1) {
        for (let j = i + 1; j < page.length; j += 1) {
          const a = page[i];
          const b = page[j];
          const apart =
            a.x + a.widthMm + settings.gapMm <= b.x + 1e-9 ||
            b.x + b.widthMm + settings.gapMm <= a.x + 1e-9 ||
            a.y + a.heightMm + settings.gapMm <= b.y + 1e-9 ||
            b.y + b.heightMm + settings.gapMm <= a.y + 1e-9;
          expect(apart).toBe(true);
        }
      }
    }
  });

  test("reports copies that cannot fit instead of shrinking them", () => {
    const huge = copy("huge", 200, 200);
    const { layout } = placements([huge]);
    expect(layout.unplaced.map((c) => c.id)).toEqual(["huge"]);
    expect(layout.pages.flat()).toHaveLength(0);
  });

  test("opens another page when the current one is full", () => {
    const copies = Array.from({ length: 12 }, (_, i) => copy(`c${i}`, 90, 90));
    const { layout } = placements(copies);
    // 2 columns x 2 rows per sheet (a third row would need 278 mm of 277).
    expect(layout.pages.length).toBe(3);
    expect(layout.pages[0]).toHaveLength(4);
    expect(layout.pages[1]).toHaveLength(4);
  });
});

describe("cut lines", () => {
  const outlined = {
    ...DEFAULT_SETTINGS,
    marginMm: 10,
    gapMm: 4,
    outlinePadMm: 3,
    outlineWidthMm: 0.4,
  };
  const room = outlined.outlinePadMm + outlined.outlineWidthMm / 2;

  test("the outline room keeps every cut line inside the margin", () => {
    const copies = Array.from({ length: 6 }, (_, i) => copy(`c${i}`, 55, 45));
    const layout = layoutSheet(copies, outlined);
    expect(layout.pages.flat()).toHaveLength(copies.length);

    for (const p of layout.pages.flat()) {
      // Box plus padding plus half the stroke: the outermost pixel drawn.
      expect(p.x - room).toBeGreaterThanOrEqual(layout.areaX - 1e-9);
      expect(p.y - room).toBeGreaterThanOrEqual(layout.areaY - 1e-9);
      expect(p.x + p.widthMm + room).toBeLessThanOrEqual(layout.areaX + layout.areaW + 1e-9);
      expect(p.y + p.heightMm + room).toBeLessThanOrEqual(layout.areaY + layout.areaH + 1e-9);
    }
  });

  test("neighbouring cut lines keep the gap apart", () => {
    const copies = Array.from({ length: 4 }, (_, i) => copy(`c${i}`, 50, 40));
    const layout = layoutSheet(copies, outlined);
    const page = layout.pages.flat();

    for (let i = 0; i < page.length; i += 1) {
      for (let j = i + 1; j < page.length; j += 1) {
        const a = page[i];
        const b = page[j];
        const apart =
          a.x + a.widthMm + room + outlined.gapMm <= b.x - room + 1e-9 ||
          b.x + b.widthMm + room + outlined.gapMm <= a.x - room + 1e-9 ||
          a.y + a.heightMm + room + outlined.gapMm <= b.y - room + 1e-9 ||
          b.y + b.heightMm + room + outlined.gapMm <= a.y - room + 1e-9;
        expect(apart).toBe(true);
      }
    }
  });

  test("without cut lines the sheet keeps the whole printable area", () => {
    const plain = { ...DEFAULT_SETTINGS, marginMm: 10, gapMm: 4, cutLines: false };
    const layout = layoutSheet([copy("a", 150, 100)], plain);
    expect(layout.pages.flat()[0].x).toBe(layout.areaX);
    expect(layout.pages.flat()[0].y).toBe(layout.areaY);
  });
});

describe("rotation", () => {
  test("a quarter turn swaps the footprint", () => {
    const upright = copy("a", 80, 40);
    expect(footprint(upright)).toEqual({ w: 80, h: 40 });
    expect(footprint(rotateCopy(upright))).toEqual({ w: 40, h: 80 });
    expect(footprint({ ...upright, rotation: 180 })).toEqual({ w: 80, h: 40 });
  });

  test("cycles through the four quarter turns", () => {
    let current = copy("a", 80, 40);
    const seen: number[] = [];
    for (let i = 0; i < 4; i += 1) {
      current = rotateCopy(current);
      seen.push(current.rotation);
    }
    expect(seen).toEqual([90, 180, 270, 0]);
  });

  test("lays out the rotated footprint and keeps the angle", () => {
    // 150 mm tall, 150 mm wide area: upright it fits, turned it still does and
    // the placement must carry the rotation and the swapped size.
    const turned = copy("turned", 120, 90, undefined, 90);
    const { layout } = placements([turned]);
    const placed = layout.pages.flat()[0];
    expect(placed.rotation).toBe(90);
    expect(placed.widthMm).toBe(90);
    expect(placed.heightMm).toBe(120);
  });

  test("a copy that only fits turned is no longer unplaced", () => {
    // Landscape 281 x 194: 250 mm tall does not fit, turned it does.
    const settings = {
      ...DEFAULT_SETTINGS,
      orientation: "landscape" as const,
      marginMm: 8,
      gapMm: 3,
    };
    const wide = copy("wide", 150, 250);
    const layout = layoutSheet([wide], settings);
    expect(layout.unplaced).toHaveLength(0);
    const placement = layout.pages[0][0];
    expect(placement.rotation).toBe(90);
    expect(placement.widthMm).toBe(250);
    expect(placement.heightMm).toBe(150);
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
