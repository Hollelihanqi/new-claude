import { describe, expect, it } from "vitest";
import {
  DEFAULT_SCHEME,
  festivalSchemeForDate,
  readInitialScheme,
  readStoredScheme,
  storeScheme,
  THEME_DEFINITIONS,
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
    expect(readStoredScheme(memoryStorage("ocean"))).toBe("ocean");
    expect(readStoredScheme(memoryStorage("sakura"))).toBe("sakura");
    expect(readStoredScheme(memoryStorage("mint"))).toBe("mint");
    expect(readStoredScheme(memoryStorage("midautumn"))).toBe("midautumn");
  });

  it("migrates the previous two accent-only themes", () => {
    expect(readStoredScheme(memoryStorage("a"))).toBe("sunset");
    expect(readStoredScheme(memoryStorage("b"))).toBe("ocean");
  });

  it("migrates the retired warm-sand style to the Apple style", () => {
    expect(readStoredScheme(memoryStorage("sand"))).toBe("apple");
  });

  it("falls back safely for missing or invalid values", () => {
    expect(readStoredScheme(memoryStorage())).toBe(DEFAULT_SCHEME);
    expect(readStoredScheme(memoryStorage("unexpected"))).toBe(DEFAULT_SCHEME);
  });

  it("stores a new selection", () => {
    const storage = memoryStorage();
    storeScheme(storage, "dragonboat");
    expect(readStoredScheme(storage)).toBe("dragonboat");
  });

  it("offers twelve daily styles and five Chinese festival skins", () => {
    expect(THEME_DEFINITIONS.filter((theme) => theme.category === "daily")).toHaveLength(12);
    expect(THEME_DEFINITIONS.filter((theme) => theme.category === "festival").map((theme) => theme.name))
      .toEqual(["新春", "元宵", "端午", "中秋", "国庆"]);
    const dailyThemes = THEME_DEFINITIONS.filter((theme) => theme.category === "daily");
    expect(dailyThemes.slice(-4).map((theme) => theme.name))
      .toEqual(["晴空", "薄荷", "柔光", "云紫"]);
    expect(dailyThemes[7]?.name).toBe("苹果");
  });

  it("maps Gregorian and Chinese-calendar festival dates to their skins", () => {
    expect(festivalSchemeForDate(new Date(2026, 1, 17, 12))).toBe("spring");
    expect(festivalSchemeForDate(new Date(2026, 2, 3, 12))).toBe("lantern");
    expect(festivalSchemeForDate(new Date(2026, 5, 19, 12))).toBe("dragonboat");
    expect(festivalSchemeForDate(new Date(2026, 8, 25, 12))).toBe("midautumn");
    expect(festivalSchemeForDate(new Date(2026, 9, 1, 12))).toBe("national");
    expect(festivalSchemeForDate(new Date(2026, 8, 19, 12))).toBeNull();
  });

  it("applies an automatic festival skin without replacing the stored preference", () => {
    const storage = memoryStorage("sakura");
    expect(readInitialScheme(storage, new Date(2026, 8, 25, 12))).toBe("midautumn");
    expect(readStoredScheme(storage)).toBe("sakura");
  });
});
