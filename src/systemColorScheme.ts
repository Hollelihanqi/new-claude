export type SystemColorScheme = "light" | "dark";

export const SYSTEM_DARK_MODE_QUERY = "(prefers-color-scheme: dark)";

type SystemSchemeMedia = Pick<MediaQueryList, "matches" | "addEventListener" | "removeEventListener">;

export function colorSchemeFromMedia(media: Pick<MediaQueryList, "matches">): SystemColorScheme {
  return media.matches ? "dark" : "light";
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
