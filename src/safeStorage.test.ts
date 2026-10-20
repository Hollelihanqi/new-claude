import { afterEach, describe, expect, it, vi } from "vitest";
import { browserStorage } from "./safeStorage";
import { readColorSchemePreference, storeColorSchemePreference } from "./systemColorScheme";
import { readStoredScheme, storeScheme } from "./themeScheme";

afterEach(() => vi.unstubAllGlobals());

describe("restricted browser storage", () => {
  it("keeps startup and preference changes usable when the storage getter throws", () => {
    vi.stubGlobal("window", Object.defineProperty({}, "localStorage", { get() { throw new Error("SecurityError"); } }));
    expect(readColorSchemePreference(browserStorage)).toBe("light");
    expect(readStoredScheme(browserStorage)).toBe("ocean");
    expect(() => storeColorSchemePreference(browserStorage, "dark")).not.toThrow();
    expect(() => storeScheme(browserStorage, "pine")).not.toThrow();
  });
  it("also tolerates individual storage operation failures", () => {
    vi.stubGlobal("window", { localStorage: { getItem() { throw new Error("denied"); }, setItem() { throw new Error("quota"); } } });
    expect(browserStorage.getItem("key")).toBeNull();
    expect(() => browserStorage.setItem("key", "value")).not.toThrow();
  });
});
