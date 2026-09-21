export type SystemColorScheme = "light" | "dark";
export type ColorSchemePreference = SystemColorScheme | "system";

export const SYSTEM_DARK_MODE_QUERY = "(prefers-color-scheme: dark)";
export const COLOR_SCHEME_PREFERENCE_KEY = "pathmux.color-scheme-preference";

type SystemSchemeMedia = Pick<MediaQueryList, "matches" | "addEventListener" | "removeEventListener">;

export function colorSchemeFromMedia(media: Pick<MediaQueryList, "matches">): SystemColorScheme {
  return media.matches ? "dark" : "light";
}

export function readColorSchemePreference(storage: Pick<Storage, "getItem">): ColorSchemePreference {
  const value = storage.getItem(COLOR_SCHEME_PREFERENCE_KEY);
  return value === "light" || value === "dark" || value === "system" ? value : "light";
}

export function preferenceForSystemFollowing(follow: boolean): ColorSchemePreference {
  return follow ? "system" : "light";
}

export function storeColorSchemePreference(
  storage: Pick<Storage, "setItem">,
  preference: ColorSchemePreference
): void {
  storage.setItem(COLOR_SCHEME_PREFERENCE_KEY, preference);
}

export function resolveColorScheme(
  preference: ColorSchemePreference,
  systemScheme: SystemColorScheme
): SystemColorScheme {
  return preference === "system" ? systemScheme : preference;
}

export function watchSystemColorScheme(
  media: SystemSchemeMedia,
  onChange: (scheme: SystemColorScheme) => void
): () => void {
  const handleChange = (event: MediaQueryListEvent) => {
    onChange(event.matches ? "dark" : "light");
  };
  media.addEventListener("change", handleChange);
  return () => media.removeEventListener("change", handleChange);
}
