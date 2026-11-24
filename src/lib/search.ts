import type { Sticker } from "../types";

/**
 * One search box, four ways to narrow: free words, `#tag`, `serie:name` and
 * `tipo:type`, plus `"exact phrases"`. Everything is compared folded (no case,
 * no accents), so `corazon` finds `corazón` and `serie:pokemon` finds `Pokémon`.
 * A free word also forgives a typo (`pikachuu` finds `Pikachu`); the prefixed
 * filters do not, because narrowing by prefix has to stay predictable.
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
  /** The words printed on the sticker, read by the AI. */
  texto: string;
}

/** Folding is done once per sticker, not once per term. */
function foldSticker(sticker: Sticker): Folded {
  return {
    name: fold(sticker.name),
    tags: sticker.tags.map(fold),
    serie: fold(sticker.serie ?? ""),
    tipo: fold(sticker.tipo ?? ""),
    texto: fold(sticker.texto ?? ""),
  };
}

/** Every wanted prefix starts the value, so typing narrows without a menu. */
function startsAll(value: string, wanted: string[]): boolean {
  return wanted.every((needle) => value.startsWith(needle));
}

/** Free words look everywhere the sticker names itself, columns included. */
function hit({ name, tags, serie, tipo, texto }: Folded, needle: string): boolean {
  return (
    name.includes(needle) ||
    serie.includes(needle) ||
    tipo.includes(needle) ||
    texto.includes(needle) ||
    tags.some((tag) => tag.includes(needle))
  );
}

/** `#tag`, `serie:`, `tipo:` and phrases stay exact; only free words are forgiven. */
function matchesColumns(folded: Folded, query: ParsedQuery): boolean {
  if (!query.active) return true;
  return (
    query.phrases.every((phrase) => hit(folded, phrase)) &&
    query.tags.every((wanted) => folded.tags.some((tag) => tag.startsWith(wanted))) &&
    (query.series.length === 0 || startsAll(folded.serie, query.series)) &&
    (query.tipos.length === 0 || startsAll(folded.tipo, query.tipos))
  );
}

/** A free word matches when it is there or one slip away; `#tag` and phrases do not. */
function termHit(folded: Folded, term: string): boolean {
  return hit(folded, term) || nearAnyWord(folded, term);
}

function matchesFolded(folded: Folded, query: ParsedQuery): boolean {
  return matchesColumns(folded, query) && query.terms.every((term) => termHit(folded, term));
}

/** One slip in a short word, two in a long one; beyond that it is another word. */
function editsAllowed(term: string): number {
  if (term.length < 4) return 0;
  return term.length >= 8 ? 2 : 1;
}

/**
 * Optimal string alignment, which counts a swapped pair (`dinsey`) as one slip.
 * Only whether the distance fits in `max` matters, so it bails out early.
 */
function nearWord(word: string, term: string, max: number): boolean {
  if (Math.abs(word.length - term.length) > max) return false;
  let before: number[] = [];
  let prev = Array.from({ length: term.length + 1 }, (_, i) => i);
  for (let i = 1; i <= word.length; i++) {
    const current = [i];
    for (let j = 1; j <= term.length; j++) {
      const cost = word[i - 1] === term[j - 1] ? 0 : 1;
      let best = Math.min(prev[j] + 1, current[j - 1] + 1, prev[j - 1] + cost);
      if (i > 1 && j > 1 && word[i - 1] === term[j - 2] && word[i - 2] === term[j - 1]) {
        best = Math.min(best, before[j - 2] + 1);
      }
      current[j] = best;
    }
    before = prev;
    prev = current;
  }
  return prev[term.length] <= max;
}

/** A typo is forgiven word by word: `pikachuu` meets «Pikachu», not a whole card. */
function nearAnyWord(folded: Folded, term: string): boolean {
  const max = editsAllowed(term);
  if (max === 0) return false;
  const words = `${folded.name} ${folded.serie} ${folded.tipo} ${folded.texto} ${folded.tags.join(" ")}`;
  return words.split(/[^\p{L}\p{N}]+/u).some((word) => word.length > 0 && nearWord(word, term, max));
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
      matchesColumns(folded, query) &&
      query.terms.every((term) => termHit(folded, term)) &&
      wanted.every((tag) => folded.tags.includes(tag)) &&
      (wantedSerie === null || folded.serie === wantedSerie) &&
      (wantedTipo === null || folded.tipo === wantedTipo)
    );
  });
}
