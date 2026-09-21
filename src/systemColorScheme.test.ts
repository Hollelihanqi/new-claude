import { describe, expect, it, vi } from "vitest";
import {
  COLOR_SCHEME_PREFERENCE_KEY,
  colorSchemeFromMedia,
  preferenceForSystemFollowing,
  readColorSchemePreference,
  resolveColorScheme,
  storeColorSchemePreference,
  watchSystemColorScheme,
} from "./systemColorScheme";

describe("system color scheme", () => {
  it("maps the macOS/browser preference to the application scheme", () => {
    expect(colorSchemeFromMedia({ matches: true })).toBe("dark");
    expect(colorSchemeFromMedia({ matches: false })).toBe("light");
  });

  it("follows preference changes and removes the listener on cleanup", () => {
    let listener: ((event: { matches: boolean }) => void) | undefined;
    const media = {
      matches: false,
      addEventListener: vi.fn((_type: string, callback: EventListener) => {
        listener = callback as unknown as (event: { matches: boolean }) => void;
      }),
      removeEventListener: vi.fn(),
    };
    const onChange = vi.fn();

    const stop = watchSystemColorScheme(media as unknown as MediaQueryList, onChange);
    listener?.({ matches: true });

    expect(onChange).toHaveBeenCalledWith("dark");
    stop();
    expect(media.removeEventListener).toHaveBeenCalledWith("change", expect.any(Function));
  });

  it("defaults to light and persists manual light, dark, or system choices", () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: vi.fn((key: string) => values.get(key) ?? null),
      setItem: vi.fn((key: string, value: string) => values.set(key, value)),
    };

    expect(readColorSchemePreference(storage)).toBe("light");
    storeColorSchemePreference(storage, "dark");
    expect(storage.setItem).toHaveBeenCalledWith(COLOR_SCHEME_PREFERENCE_KEY, "dark");
    expect(readColorSchemePreference(storage)).toBe("dark");
    expect(resolveColorScheme("system", "light")).toBe("light");
    expect(resolveColorScheme("system", "dark")).toBe("dark");
    expect(resolveColorScheme("light", "dark")).toBe("light");
  });

  it("only follows the system when explicitly enabled and returns to light when disabled", () => {
    expect(preferenceForSystemFollowing(true)).toBe("system");
    expect(preferenceForSystemFollowing(false)).toBe("light");
    expect(resolveColorScheme(preferenceForSystemFollowing(false), "dark")).toBe("light");
  });
});
