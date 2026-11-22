import type { Sticker } from "../types";

/**
 * One search box, four ways to narrow: free words, `#tag`, `serie:name` and
 * `tipo:type`, plus `"exact phrases"`. Everything is compared folded (no case,
 * no accents), so `corazon` finds `corazón` and `serie:pokemon` finds `Pokémon`.
 */
export interface ParsedQuery {
  /** Free words: each one must appear in the name or in some tag. */
  terms: string[];
  /** `#tag` / `tag:` — a tag that starts with this. */
  tags: string[];
  /** `serie:name` / `@name` — the series column, which starts with this. */
  series: string[];
  /** `tipo:type` — the type column, which starts with this. */
  tipos: string[];
  /** `"exact phrase"` — anywhere in the name or in some tag. */
  phrases: string[];
  active: boolean;
}

const TOKEN =
  /(?:#|tag:)(?<tag>\S+)|(?:serie:|@)(?<serie>\S+)|tipo:(?<tipo>\S+)|"(?<phrase>[^"]*)"|(?<word>\S+)/gi;

/** A token with no letter or digit («#», «@», «-») is not a filter. */
const MEANINGFUL = /[\p{L}\p{N}]/u;

/** «tag:» or «serie:» on their own, with nothing after them yet. */
const BARE_PREFIX = /^(?:tag|serie|tipo):$/i;

/** Lowercase without diacritics: the library has «corazón» and «Pokémon». */
export function fold(value: string): string {
  return value
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase();
}

export function parseQuery(raw: string): ParsedQuery {
  const terms: string[] = [];
  const tags: string[] = [];
  const series: string[] = [];
  const tipos: string[] = [];
  const phrases: string[] = [];

  for (const match of raw.matchAll(TOKEN)) {
    const groups = match.groups ?? {};
    if (groups.tag) {
      const value = fold(groups.tag);
      if (MEANINGFUL.test(value)) tags.push(value);
    } else if (groups.serie) {
      const value = fold(groups.serie);
      if (MEANINGFUL.test(value)) series.push(value);
    } else if (groups.tipo) {
      const value = fold(groups.tipo);
      if (MEANINGFUL.test(value)) tipos.push(value);
    } else if (groups.phrase !== undefined) {
      // A pair of quotes with nothing in between is not a filter.
      const value = fold(groups.phrase.trim());
      if (MEANINGFUL.test(value)) phrases.push(value);
    } else if (groups.word) {
      // An opening quote with no closing one still searches for the word.
      const word = groups.word.replace(/"/g, "");
      const value = fold(word);
      if (!BARE_PREFIX.test(word) && MEANINGFUL.test(value)) terms.push(value);
    }
  }

  return {
    terms,
    tags,
    series,
    tipos,
    phrases,
    active:
      terms.length + tags.length + series.length + tipos.length + phrases.length > 0,
  };
}

interface Folded {
  name: string;
  tags: string[];
  serie: string;
  tipo: string;
}

/** Folding is done once per sticker, not once per term. */
function foldSticker(sticker: Sticker): Folded {
  return {
    name: fold(sticker.name),
    tags: sticker.tags.map(fold),
    serie: fold(sticker.serie ?? ""),
    tipo: fold(sticker.tipo ?? ""),
  };
}

/** Every wanted prefix starts the value, so typing narrows without a menu. */
function startsAll(value: string, wanted: string[]): boolean {
  return wanted.every((needle) => value.startsWith(needle));
}

function matchesFolded({ name, tags, serie, tipo }: Folded, query: ParsedQuery): boolean {
  if (!query.active) return true;
  // Free words look everywhere the sticker names itself, columns included.
  const anywhere = (needle: string) =>
    name.includes(needle) ||
    serie.includes(needle) ||
    tipo.includes(needle) ||
    tags.some((tag) => tag.includes(needle));
  const startsATag = (wanted: string) => tags.some((tag) => tag.startsWith(wanted));

  return (
    query.phrases.every(anywhere) &&
    query.terms.every(anywhere) &&
    query.tags.every(startsATag) &&
    (query.series.length === 0 || startsAll(serie, query.series)) &&
    (query.tipos.length === 0 || startsAll(tipo, query.tipos))
  );
}

/** The sticker's tags in their original spelling, for the matching chips. */
export function matchedTags(sticker: Sticker, query: ParsedQuery): string[] {
  if (!query.active) return [];
  return sticker.tags.filter((tag) => {
    const value = fold(tag);
    return (
      query.tags.some((wanted) => value.startsWith(wanted)) ||
      query.phrases.some((wanted) => value.includes(wanted)) ||
      query.terms.some((wanted) => value.includes(wanted))
    );
  });
}

/** The series or the type that made a `serie:` / `tipo:` search match. */
export interface FacetMatch {
  kind: "serie" | "tipo";
  /** The value in its canonical spelling, as it is shown on the card. */
  text: string;
}

/** `serie:`/`tipo:` ask for a prefix; a free word only has to appear inside. */
function facetHit(value: string, wanted: string[], query: ParsedQuery): boolean {
  if (wanted.length > 0) return startsAll(value, wanted);
  return (
    query.terms.some((term) => value.includes(term)) ||
    query.phrases.some((phrase) => value.includes(phrase))
  );
}

export function matchedFacets(sticker: Sticker, query: ParsedQuery): FacetMatch[] {
  if (!query.active) return [];
  const found: FacetMatch[] = [];
  const serie = sticker.serie ?? "";
  if (serie && facetHit(fold(serie), query.series, query)) {
    found.push({ kind: "serie", text: serie });
  }
  const tipo = sticker.tipo ?? "";
  if (tipo && facetHit(fold(tipo), query.tipos, query)) {
    found.push({ kind: "tipo", text: tipo });
  }
  return found;
}

export function stickerMatches(sticker: Sticker, query: ParsedQuery): boolean {
  return matchesFolded(foldSticker(sticker), query);
}

/** Everything the sidebar and the search box add on top of the whole library. */
export interface LibraryFilters {
  tags?: string[];
  /** One series, or "" for the stickers still without one. */
  serie?: string | null;
  /** One type, or "" for the stickers still without one. */
  tipo?: string | null;
}

/** Text search, tag chips and the series/type facets narrow the same list. */
export function filterStickers(
  items: Sticker[],
  query: ParsedQuery,
  filters: LibraryFilters = {},
): Sticker[] {
  const tags = filters.tags ?? [];
  const serie = filters.serie ?? null;
  const tipo = filters.tipo ?? null;
  if (!query.active && tags.length === 0 && serie === null && tipo === null) return items;

  const wanted = tags.map(fold);
  const wantedSerie = serie === null ? null : fold(serie);
  const wantedTipo = tipo === null ? null : fold(tipo);

  return items.filter((sticker) => {
    const folded = foldSticker(sticker);
    return (
      matchesFolded(folded, query) &&
      wanted.every((tag) => folded.tags.includes(tag)) &&
      (wantedSerie === null || folded.serie === wantedSerie) &&
      (wantedTipo === null || folded.tipo === wantedTipo)
    );
  });
}
