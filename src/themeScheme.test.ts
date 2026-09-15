import { describe, expect, it } from "vitest";
import {
  readStoredScheme,
  storeScheme,
  THEME_SCHEME_STORAGE_KEY,
} from "./themeScheme";

function memoryStorage(initial?: string) {
  const values = new Map<string, string>();
  if (initial !== undefined) values.set(THEME_SCHEME_STORAGE_KEY, initial);
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  };
}

describe("theme scheme persistence", () => {
  it("restores the previously selected scheme", () => {
    expect(readStoredScheme(memoryStorage("a"))).toBe("a");
    expect(readStoredScheme(memoryStorage("b"))).toBe("b");
  });

  it("falls back safely for missing or invalid values", () => {
    expect(readStoredScheme(memoryStorage())).toBe("b");
    expect(readStoredScheme(memoryStorage("unexpected"))).toBe("b");
  });

  it("stores a new selection", () => {
    const storage = memoryStorage();
    storeScheme(storage, "a");
    expect(readStoredScheme(storage)).toBe("a");
  });
});
