export type ContourPolygon = [number, number][];

/// Shape used when a silhouette could not be traced: the same rectangle the
/// PDF writer draws.
export const RECTANGLE_CONTOUR: ContourPolygon[] = [
  [
    [0, 0],
    [1, 0],
    [1, 1],
    [0, 1],
  ],
];

// Traced silhouettes survive leaving and re-entering the sheet, so returning to
// a big sheet does not redo every trace. Failures are remembered separately:
// they show a rectangle for now, but the next visit retries them.
const cache = new Map<string, ContourPolygon[]>();
const failed = new Set<string>();

export function cachedContours(): Record<string, ContourPolygon[]> {
  return Object.fromEntries(cache);
}

export function rememberContour(key: string, shape: ContourPolygon[]): void {
  cache.set(key, shape);
  failed.delete(key);
}

export function rememberFailure(key: string): void {
  cache.set(key, RECTANGLE_CONTOUR);
  failed.add(key);
}

/** Drops the cached rectangles so the next render traces those keys again. */
export function retryFailedContours(): number {
  const retried = failed.size;
  for (const key of failed) cache.delete(key);
  failed.clear();
  return retried;
}

export function clearContourCache(): void {
  cache.clear();
  failed.clear();
}
