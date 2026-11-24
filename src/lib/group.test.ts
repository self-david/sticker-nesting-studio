import { describe, expect, test } from "bun:test";
import type { Sticker } from "../types";
import { groupStickers, visualOrder } from "./group";

function sticker(name: string, serie: string, tipo: string): Sticker {
  return {
    id: name,
    name,
    ext: "png",
    kind: "raster",
    widthPx: 10,
    heightPx: 10,
    widthMm: 10,
    heightMm: 10,
    dpi: 300,
    hasAlpha: true,
    sizeBytes: 1,
    createdAt: 0,
    fileName: `${name}.png`,
    filePath: "",
    thumbPath: "",
    tags: [],
    serie,
    tipo,
    aiAt: 0,
    classifiedAt: 1,
    texto: "",
  };
}

const all = [
  sticker("Pikachu", "Pokémon", "videojuego"),
  sticker("Eevee", "Pokémon", "videojuego"),
  sticker("Naruto", "Naruto", "anime"),
  sticker("path1", "", "otros"),
  sticker("Mandala", "", "cotidiano"),
];

describe("groupStickers", () => {
  test("agrupa por serie con el grupo mayor primero", () => {
    const groups = groupStickers(all, "serie");
    expect(groups.map((g) => g.key)).toEqual(["Pokémon", "Naruto", ""]);
    expect(groups[0].items).toHaveLength(2);
  });

  test("el grupo vacío queda al final", () => {
    const groups = groupStickers(all, "serie");
    expect(groups.at(-1)?.key).toBe("");
    expect(groups.at(-1)?.items).toHaveLength(2);
  });

  test("no pierde ni un sticker", () => {
    for (const by of ["serie", "tipo"] as const) {
      const total = groupStickers(all, by).reduce((sum, group) => sum + group.items.length, 0);
      expect(total).toBe(all.length);
    }
  });

  test("dentro de un grupo se respeta el orden de entrada", () => {
    expect(groupStickers(all, "serie")[0].items.map((s) => s.name)).toEqual(["Pikachu", "Eevee"]);
  });

  test("el orden visual es el de los grupos, no el de la lista original", () => {
    const order = visualOrder(groupStickers(all, "serie")).map((s) => s.name);
    expect(order).toEqual(["Pikachu", "Eevee", "Naruto", "path1", "Mandala"]);
  });
});
