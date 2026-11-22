import { describe, expect, test } from "bun:test";
import type { Sticker } from "../types";
import {
  filterStickers,
  fold,
  matchedFacets,
  matchedTags,
  parseQuery,
  stickerMatches,
} from "./search";

function sticker(name: string, tags: string[], serie = "", tipo = ""): Sticker {
  return {
    id: name,
    name,
    ext: "png",
    kind: "raster",
    widthPx: 100,
    heightPx: 100,
    widthMm: 25,
    heightMm: 25,
    dpi: 300,
    hasAlpha: true,
    sizeBytes: 1000,
    createdAt: 0,
    fileName: `${name}.png`,
    filePath: "",
    thumbPath: "",
    tags,
    serie,
    tipo,
    aiAt: 0,
    classifiedAt: 0,
  };
}

const kitty = sticker("Hello Kitty con corona", ["personaje", "sanrio", "gato", "corazón"]);
const tiger = sticker("Cachorro de tigre", ["animal", "tigre", "disney"]);
const bunny = sticker("Conor el conejo", ["personaje", "conejo", "pascua"]);
const simba = sticker("Simba", ["animal", "león"], "Disney", "cine");
const pikachu = sticker("Pikachu", ["ratón"], "Pokémon", "anime");
const dragon = sticker("Goku", ["saiyan"], "Dragon Ball", "anime");

describe("fold", () => {
  test("quita mayúsculas y acentos", () => {
    expect(fold("Corazón")).toBe("corazon");
    expect(fold("POKÉMON")).toBe("pokemon");
    expect(fold("Niño")).toBe("nino");
  });
});

describe("parseQuery", () => {
  test("palabras sueltas", () => {
    const q = parseQuery("gato azul");
    expect(q.terms).toEqual(["gato", "azul"]);
    expect(q.active).toBe(true);
  });

  test("#etiqueta y tag:", () => {
    const q = parseQuery("#gato tag:Sanrio");
    expect(q.tags).toEqual(["gato", "sanrio"]);
    expect(q.terms).toEqual([]);
  });

  test("serie: y @", () => {
    const q = parseQuery("serie:disney @pokemon");
    expect(q.series).toEqual(["disney", "pokemon"]);
  });

  test("tipo: es su propio filtro", () => {
    const q = parseQuery("tipo:anime tipo:cine");
    expect(q.tipos).toEqual(["anime", "cine"]);
    expect(q.terms).toEqual([]);
    expect(q.series).toEqual([]);
  });

  test("frases entre comillas conviven con palabras", () => {
    const q = parseQuery('"mujer con corona" azul');
    expect(q.phrases).toEqual(["mujer con corona"]);
    expect(q.terms).toEqual(["azul"]);
  });

  test("una comilla sin cerrar sigue buscando la palabra", () => {
    expect(parseQuery('"corona').terms).toEqual(["corona"]);
  });

  test("vacío o solo comillas no filtra", () => {
    expect(parseQuery("   ").active).toBe(false);
    expect(parseQuery('""').active).toBe(false);
  });

  test("un prefijo a medio escribir no vacía la biblioteca", () => {
    expect(parseQuery("#").active).toBe(false);
    expect(parseQuery("@").active).toBe(false);
    expect(parseQuery("tag:").active).toBe(false);
    expect(parseQuery("serie:").active).toBe(false);
    expect(parseQuery("tipo:").active).toBe(false);
    expect(parseQuery("-").active).toBe(false);
  });
});

describe("stickerMatches", () => {
  test("busca en el nombre sin acentos ni mayúsculas", () => {
    expect(stickerMatches(kitty, parseQuery("CORAZON"))).toBe(true);
    expect(stickerMatches(kitty, parseQuery("kitty"))).toBe(true);
  });

  test("busca dentro de las etiquetas", () => {
    expect(stickerMatches(kitty, parseQuery("sanrio"))).toBe(true);
    expect(stickerMatches(tiger, parseQuery("sanrio"))).toBe(false);
  });

  test("varios términos son un AND", () => {
    expect(stickerMatches(kitty, parseQuery("kitty gato"))).toBe(true);
    expect(stickerMatches(kitty, parseQuery("kitty tigre"))).toBe(false);
  });

  test("#etiqueta exige que la etiqueta empiece por el texto", () => {
    expect(stickerMatches(kitty, parseQuery("#gato"))).toBe(true);
    expect(stickerMatches(tiger, parseQuery("#gato"))).toBe(false);
    // El nombre contiene «corona» pero no hay etiqueta que empiece así.
    expect(stickerMatches(kitty, parseQuery("#corona"))).toBe(false);
  });

  test("serie: busca en la columna de serie, no en las etiquetas", () => {
    // La etiqueta «disney» no basta: este sticker no tiene serie.
    expect(stickerMatches(tiger, parseQuery("serie:disney"))).toBe(false);
    expect(stickerMatches(simba, parseQuery("serie:Disney"))).toBe(true);
    // Se puede escribir a medias y sigue encontrando «Pokémon».
    expect(stickerMatches(pikachu, parseQuery("serie:pokemon"))).toBe(true);
    expect(stickerMatches(pikachu, parseQuery("serie:pok"))).toBe(true);
    expect(stickerMatches(pikachu, parseQuery("serie:dragon"))).toBe(false);
  });

  test("tipo: busca en la columna de tipo", () => {
    expect(stickerMatches(pikachu, parseQuery("tipo:anime"))).toBe(true);
    expect(stickerMatches(simba, parseQuery("tipo:cine"))).toBe(true);
    expect(stickerMatches(simba, parseQuery("tipo:anime"))).toBe(false);
    // Los que aún no tienen tipo no caen en ningún tipo escrito.
    expect(stickerMatches(bunny, parseQuery("tipo:anime"))).toBe(false);
  });

  test("serie y tipo se suman al texto", () => {
    expect(stickerMatches(dragon, parseQuery("tipo:anime serie:dragon"))).toBe(true);
    expect(stickerMatches(pikachu, parseQuery("tipo:anime serie:dragon"))).toBe(false);
    expect(stickerMatches(dragon, parseQuery("goku tipo:anime"))).toBe(true);
  });

  test("el texto libre también busca en serie y tipo", () => {
    expect(stickerMatches(simba, parseQuery("disney"))).toBe(true);
    expect(stickerMatches(simba, parseQuery("cine"))).toBe(true);
    expect(stickerMatches(pikachu, parseQuery("anime"))).toBe(true);
    expect(stickerMatches(pikachu, parseQuery("disney"))).toBe(false);
    expect(stickerMatches(simba, parseQuery('"walt disney"'))).toBe(false);
    expect(stickerMatches(simba, parseQuery('"disney"'))).toBe(true);
  });

  test("las frases buscan el texto completo", () => {
    expect(stickerMatches(kitty, parseQuery('"hello kitty"'))).toBe(true);
    expect(stickerMatches(kitty, parseQuery('"kitty hello"'))).toBe(false);
  });
});

describe("matchedTags", () => {
  test("devuelve las etiquetas en su escritura original", () => {
    expect(matchedTags(kitty, parseQuery("corazon"))).toEqual(["corazón"]);
    expect(matchedTags(kitty, parseQuery("#gato"))).toEqual(["gato"]);
  });

  test("sin consulta no resalta nada", () => {
    expect(matchedTags(kitty, parseQuery(""))).toEqual([]);
  });

  test("serie: y tipo: no encienden etiquetas", () => {
    expect(matchedTags(tiger, parseQuery("serie:disney"))).toEqual([]);
    expect(matchedTags(simba, parseQuery("serie:disney tipo:cine"))).toEqual([]);
  });
});

describe("matchedFacets", () => {
  test("resalta la serie en su escritura canónica", () => {
    expect(matchedFacets(pikachu, parseQuery("serie:pokemon"))).toEqual([
      { kind: "serie", text: "Pokémon" },
    ]);
  });

  test("resalta el tipo tal como está guardado", () => {
    expect(matchedFacets(simba, parseQuery("tipo:cine"))).toEqual([{ kind: "tipo", text: "cine" }]);
  });

  test("serie y tipo juntos, en ese orden", () => {
    expect(matchedFacets(simba, parseQuery("tipo:cine serie:dis"))).toEqual([
      { kind: "serie", text: "Disney" },
      { kind: "tipo", text: "cine" },
    ]);
  });

  test("no resalta nada si el sticker no coincide o no hay consulta", () => {
    expect(matchedFacets(tiger, parseQuery("serie:disney"))).toEqual([]);
    expect(matchedFacets(simba, parseQuery(""))).toEqual([]);
    expect(matchedFacets(simba, parseQuery("simba"))).toEqual([]);
  });

  test("una palabra suelta que coincide con la serie o el tipo también se resalta", () => {
    expect(matchedFacets(simba, parseQuery("disney"))).toEqual([
      { kind: "serie", text: "Disney" },
    ]);
    expect(matchedFacets(pikachu, parseQuery("anim"))).toEqual([{ kind: "tipo", text: "anime" }]);
  });
});

describe("filterStickers", () => {
  const all = [kitty, tiger, bunny];

  test("sin filtros devuelve la misma lista", () => {
    expect(filterStickers(all, parseQuery(""), {})).toBe(all);
  });

  test("combina texto y etiquetas activas del panel", () => {
    expect(filterStickers(all, parseQuery("personaje"), { tags: ["conejo"] })).toEqual([bunny]);
    expect(filterStickers(all, parseQuery(""), { tags: ["personaje", "disney"] })).toEqual([]);
  });

  test("el filtro de etiquetas no distingue acentos", () => {
    expect(filterStickers(all, parseQuery(""), { tags: ["corazon"] })).toEqual([kitty]);
  });

  test("filtra por serie y tipo, y el vacío es un cubo propio", () => {
    const classified = [
      { ...kitty, serie: "Sanrio", tipo: "kawaii" },
      { ...tiger, serie: "Disney", tipo: "cartoon" },
      bunny,
    ];
    expect(filterStickers(classified, parseQuery(""), { serie: "Sanrio" })).toEqual([
      classified[0],
    ]);
    expect(filterStickers(classified, parseQuery(""), { serie: "" })).toEqual([bunny]);
    expect(filterStickers(classified, parseQuery(""), { tipo: "cartoon" })).toEqual([
      classified[1],
    ]);
    expect(filterStickers(classified, parseQuery(""), { tipo: "" })).toEqual([bunny]);
  });

  test("serie y tipo se acumulan con el texto y las etiquetas", () => {
    const classified = [
      { ...kitty, serie: "Sanrio", tipo: "kawaii" },
      { ...tiger, serie: "Disney", tipo: "cartoon" },
    ];
    expect(
      filterStickers(classified, parseQuery("kitty"), { serie: "Sanrio", tags: ["gato"] }),
    ).toEqual([classified[0]]);
    expect(
      filterStickers(classified, parseQuery("kitty"), { serie: "Sanrio", tipo: "cartoon" }),
    ).toEqual([]);
  });

  test("serie: y tipo: también filtran la lista, junto al panel", () => {
    const classified = [pikachu, dragon, simba];
    expect(filterStickers(classified, parseQuery("serie:dragon"))).toEqual([dragon]);
    expect(filterStickers(classified, parseQuery("tipo:anime"))).toEqual([pikachu, dragon]);
    expect(filterStickers(classified, parseQuery("tipo:anime"), { serie: "Pokémon" })).toEqual([
      pikachu,
    ]);
  });
});
