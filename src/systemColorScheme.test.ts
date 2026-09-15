import { describe, expect, it, vi } from "vitest";
import { colorSchemeFromMedia, watchSystemColorScheme } from "./systemColorScheme";

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
});
