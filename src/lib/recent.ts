/**
 * The sheet that was last saved or opened, kept in the webview's local storage
 * so the app can reopen it on the next start. Only a path is stored: the
 * artwork lives inside the `.sns` file.
 */
const LAST_SHEET_KEY = "sns.last-sheet";
/** The sheet kept inside the app. Only one of the two is ever stored: the one
 * used last is the one the app reopens. */
const LAST_LOCAL_KEY = "sns.last-local-sheet";

export function lastSheet(): string | null {
  try {
    const value = localStorage.getItem(LAST_SHEET_KEY);
    return value && value.trim() ? value : null;
  } catch {
    return null;
  }
}

export function rememberLastSheet(path: string): void {
  try {
    if (!path.trim()) return;
    localStorage.setItem(LAST_SHEET_KEY, path);
    localStorage.removeItem(LAST_LOCAL_KEY);
  } catch {
    // A full or disabled storage only costs the automatic reopen.
  }
}

export function forgetLastSheet(): void {
  try {
    localStorage.removeItem(LAST_SHEET_KEY);
  } catch {
    // Nothing to clean up if storage is unavailable.
  }
}

export function lastLocalSheet(): string | null {
  try {
    const value = localStorage.getItem(LAST_LOCAL_KEY);
    return value && value.trim() ? value : null;
  } catch {
    return null;
  }
}

export function rememberLastLocalSheet(id: string): void {
  try {
    if (!id.trim()) return;
    localStorage.setItem(LAST_LOCAL_KEY, id);
    localStorage.removeItem(LAST_SHEET_KEY);
  } catch {
    // A full or disabled storage only costs the automatic reopen.
  }
}

export function forgetLastLocalSheet(): void {
  try {
    localStorage.removeItem(LAST_LOCAL_KEY);
  } catch {
    // Nothing to clean up if storage is unavailable.
  }
}
