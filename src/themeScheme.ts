export type Scheme = "a" | "b";

export const THEME_SCHEME_STORAGE_KEY = "pathmux.theme-scheme";

type ThemeStorage = Pick<Storage, "getItem" | "setItem">;

export function readStoredScheme(storage: ThemeStorage): Scheme {
  try {
    return storage.getItem(THEME_SCHEME_STORAGE_KEY) === "a" ? "a" : "b";
  } catch {
    return "b";
  }
}

export function storeScheme(storage: ThemeStorage, scheme: Scheme): void {
  try {
    storage.setItem(THEME_SCHEME_STORAGE_KEY, scheme);
  } catch {
    // localStorage 被系统策略禁用时，当前会话仍可正常切换主题。
  }
}
