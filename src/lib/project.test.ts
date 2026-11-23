import { describe, expect, test } from "bun:test";
import { DEFAULT_SETTINGS, MIN_PAPER_MM, type SheetCopy } from "./sheet";
import { hydrateProject, normalizeAngle, projectCopies } from "./project";

const copy = (id: string): SheetCopy => ({
  id,
  stickerId: `s-${id}`,
  name: `Sticker ${id}`,
  ext: "png",
  filePath: `C:/lib/${id}.png`,
  thumbPath: `C:/lib/thumbs/${id}.png`,
  widthMm: 42,
  heightMm: 21,
  aspect: 2,
  rotation: 90,
});

describe("projectCopies", () => {
  test("keeps the layout of every copy, paths included", () => {
    const [saved] = projectCopies([copy("a")]);
    expect(saved.filePath).toBe("C:/lib/a.png");
    expect(saved.thumbPath).toBe("C:/lib/thumbs/a.png");
    expect(saved.widthMm).toBe(42);
    expect(saved.rotation).toBe(90);
  });

  test("falls back to the thumbnail when the original path is gone", () => {
    const [saved] = projectCopies([{ ...copy("b"), filePath: "" }]);
    expect(saved.filePath).toBe("C:/lib/thumbs/b.png");
  });
});

describe("hydrateProject", () => {
  const payload = {
    settings: { ...DEFAULT_SETTINGS, paper: "letter" as const, gapMm: 5 },
    copies: [
      {
        id: "a",
        stickerId: "s-a",
        name: "Sticker a",
        ext: "png",
        filePath: "C:/sheets/hoja-stickers/archivos/a.png",
        thumbPath: "C:/sheets/hoja-stickers/miniaturas/a.png",
        widthMm: 42,
        heightMm: 21,
        aspect: 2,
        rotation: 270,
      },
    ],
  };

  test("restores the sheet settings and copies", () => {
    const loaded = hydrateProject(payload);
    expect(loaded.settings.paper).toBe("letter");
    expect(loaded.settings.gapMm).toBe(5);
    expect(loaded.copies).toHaveLength(1);
    expect(loaded.copies[0].filePath).toContain("archivos/a.png");
    expect(loaded.copies[0].rotation).toBe(270);
  });

  test("keeps a custom page size and clamps what is out of range", () => {
    const loaded = hydrateProject({
      ...payload,
      settings: {
        ...payload.settings,
        paper: "custom" as never,
        customWidthMm: 330 as never,
        customHeightMm: 5 as never,
      },
    });
    expect(loaded.settings.paper).toBe("custom");
    expect(loaded.settings.customWidthMm).toBe(330);
    // Anything smaller than a printable page falls back to the lower bound.
    expect(loaded.settings.customHeightMm).toBe(MIN_PAPER_MM);
  });

  test("falls back to A4 for an unknown paper", () => {
    const loaded = hydrateProject({
      ...payload,
      settings: { ...payload.settings, paper: "a3" as never },
    });
    expect(loaded.settings.paper).toBe("a4");
  });

  test("clamps sizes and skips copies without a file", () => {
    const loaded = hydrateProject({
      ...payload,
      copies: [
        { ...payload.copies[0], id: "big", widthMm: 99_999 },
        { ...payload.copies[0], id: "lost", filePath: "", thumbPath: "" },
      ],
    });
    expect(loaded.copies).toHaveLength(1);
    expect(loaded.copies[0].widthMm).toBe(2000);
    expect(loaded.copies[0].aspect).toBe(2);
  });

  test("rounds rotation to quarter turns", () => {
    expect(normalizeAngle(45)).toBe(45);
    expect(normalizeAngle(60)).toBe(45);
    expect(normalizeAngle(-90)).toBe(270);
    expect(normalizeAngle(450)).toBe(90);
    expect(normalizeAngle(Number.NaN)).toBe(0);
  });
});
