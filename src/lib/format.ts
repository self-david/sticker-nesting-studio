export function fmtMm(value: number, locale: string): string {
  return new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(value);
}

export function fmtBytes(bytes: number): string {
  if (!bytes) return "0 KB";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
  const value = bytes / Math.pow(1024, i);
  return `${value.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

export function fmtDate(ms: number, locale: string): string {
  return new Intl.DateTimeFormat(locale, { dateStyle: "medium" }).format(new Date(ms));
}

export function round1(value: number): number {
  return Math.round(value * 10) / 10;
}
