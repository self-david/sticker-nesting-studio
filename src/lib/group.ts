import type { Sticker } from "../types";

export type GroupBy = "serie" | "tipo";

export interface Group {
  /** Canonical value; "" is the «no series» / «no type» bucket. */
  key: string;
  items: Sticker[];
}

/**
 * Groups the visible stickers by one facet. The biggest group comes first and
 * the empty value goes last, so the ones still to classify are always at hand.
 */
export function groupStickers(items: Sticker[], by: GroupBy): Group[] {
  const buckets = new Map<string, Sticker[]>();
  for (const sticker of items) {
    const key = sticker[by];
    const bucket = buckets.get(key);
    if (bucket) bucket.push(sticker);
    else buckets.set(key, [sticker]);
  }

  return [...buckets.entries()]
    .map(([key, groupItems]) => ({ key, items: groupItems }))
    .sort((a, b) => {
      if (a.key === "") return 1;
      if (b.key === "") return -1;
      return b.items.length - a.items.length || a.key.localeCompare(b.key, "es");
    });
}

/** The order the grid paints: shift-click walks through this, not the raw list. */
export function visualOrder(groups: Group[]): Sticker[] {
  return groups.flatMap((group) => group.items);
}
