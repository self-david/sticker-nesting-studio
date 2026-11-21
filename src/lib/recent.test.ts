import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { forgetLastSheet, lastSheet, rememberLastSheet } from "./recent";

const store = new Map<string, string>();

function stubStorage(failing = false) {
  (globalThis as { localStorage?: Storage }).localStorage = {
    getItem: (key: string) => (failing ? null : (store.get(key) ?? null)),
    setItem: (key: string, value: string) => {
      if (failing) throw new Error("storage unavailable");
      store.set(key, value);
    },
    removeItem: (key: string) => {
      if (failing) throw new Error("storage unavailable");
      store.delete(key);
    },
    clear: () => store.clear(),
    key: () => null,
    length: 0,
  };
}

beforeEach(() => {
  store.clear();
  stubStorage();
});

afterEach(() => {
  delete (globalThis as { localStorage?: Storage }).localStorage;
});

describe("last sheet", () => {
  test("remembers the path across launches", () => {
    expect(lastSheet()).toBeNull();
    rememberLastSheet("C:/hojas/hoja.sns");
    expect(lastSheet()).toBe("C:/hojas/hoja.sns");
  });

  test("forgetting leaves nothing to restore", () => {
    rememberLastSheet("C:/hojas/hoja.sns");
    forgetLastSheet();
    expect(lastSheet()).toBeNull();
  });

  test("ignores a blank path instead of overwriting the good one", () => {
    rememberLastSheet("C:/hojas/hoja.sns");
    rememberLastSheet("   ");
    expect(lastSheet()).toBe("C:/hojas/hoja.sns");
  });

  test("storage failures only cost the automatic reopen", () => {
    stubStorage(true);
    expect(() => rememberLastSheet("C:/hojas/hoja.sns")).not.toThrow();
    expect(() => forgetLastSheet()).not.toThrow();
    expect(lastSheet()).toBeNull();
  });
});
