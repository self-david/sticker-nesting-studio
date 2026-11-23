import { describe, expect, test } from "bun:test";
import { layoutStats, type SheetLayout } from "./nest";
import type { SheetCopy } from "./sheet";

const copy = (id: string): SheetCopy => ({
  id,
  stickerId: `s-${id}`,
  name: `Sticker ${id}`,
  ext: "png",
  filePath: `C:/lib/${id}.png`,
  thumbPath: `C:/lib/thumbs/${id}.png`,
  widthMm: 100,
  heightMm: 100,
  aspect: 1,
  rotation: 0,
});

const layout = (inkMm2: number): SheetLayout => ({
  pageW: 210,
  pageH: 297,
  areaX: 4,
  areaY: 4,
  areaW: 202,
  areaH: 289,
  pages: [
    [
      { copy: copy("a"), x: 4, y: 4, widthMm: 100, heightMm: 100, rotation: 0, artX: 0, artY: 0 },
      { copy: copy("b"), x: 108, y: 4, widthMm: 100, heightMm: 100, rotation: 0, artX: 0, artY: 0 },
    ],
  ],
  unplaced: [],
  inkMm2,
});

describe("sheet stats", () => {
  test("reports ink and box separately over the printable area", () => {
    const stats = layoutStats(layout(12_000));
    const available = 202 * 289;
    expect(stats.placed).toBe(2);
    // 2 × 100 × 100 mm² of artwork, whatever the silhouettes are worth.
    expect(stats.box).toBeCloseTo(20_000 / available, 6);
    expect(stats.ink).toBeCloseTo(12_000 / available, 6);
  });

  test("a turned copy counts its turned box, like the old rectangle layout", () => {
    const turned = layout(0);
    turned.pages[0][0].rotation = 45;
    const wide = 100 * Math.SQRT2;
    expect(layoutStats(turned).box).toBeCloseTo(
      (wide * wide + 100 * 100) / (202 * 289),
      6,
    );
  });

  test("ink never counts more than the boxes it sits in", () => {
    expect(layoutStats(layout(0)).ink).toBe(0);
    expect(layoutStats(layout(20_000)).ink).toBeCloseTo(
      layoutStats(layout(20_000)).box,
      6,
    );
  });

  test("an empty sheet has nothing to fill", () => {
    const empty: SheetLayout = { ...layout(0), pages: [[]] };
    expect(layoutStats(empty)).toEqual({ placed: 0, ink: 0, box: 0 });
  });
});
