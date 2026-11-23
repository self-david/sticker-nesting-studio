/**
 * How many CSS pixels one millimetre takes on this screen. Screens lie about
 * their physical size, so the number is calibrated by the user: the sheet is
 * then drawn at real size and a sticker measures what it measures on paper.
 */
const KEY = "sns.px-per-mm";

/** 96 CSS px per inch, the usual starting point. */
export const DEFAULT_PX_PER_MM = 96 / 25.4;
export const MIN_PX_PER_MM = 1;
export const MAX_PX_PER_MM = 40;

/** The ruler the calibration is set against, in centimetres. */
export const RULER_CM = 10;

export function pxPerMm(): number {
  try {
    const raw = Number(localStorage.getItem(KEY));
    if (Number.isFinite(raw) && raw >= MIN_PX_PER_MM && raw <= MAX_PX_PER_MM) return raw;
  } catch {
    // Storage disabled: the default is still a usable estimate.
  }
  return DEFAULT_PX_PER_MM;
}

export function rememberPxPerMm(value: number): void {
  try {
    if (!Number.isFinite(value)) return;
    const next = Math.min(MAX_PX_PER_MM, Math.max(MIN_PX_PER_MM, value));
    localStorage.setItem(KEY, String(next));
  } catch {
    // A full or disabled storage only loses the calibration.
  }
}
