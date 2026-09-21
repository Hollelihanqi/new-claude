export type Scheme =
  | "ocean"
  | "glacier"
  | "graphite"
  | "pine"
  | "sunset"
  | "iris"
  | "sakura"
  | "sky"
  | "mint"
  | "peach"
  | "lavender"
  | "apple"
  | "spring"
  | "lantern"
  | "dragonboat"
  | "midautumn"
  | "national";

export type ThemeCategory = "daily" | "festival";

export type ThemeDefinition = {
  value: Scheme;
  name: string;
  description: string;
  category: ThemeCategory;
  colors: readonly [string, string, string, string, string];
};

export const THEME_SCHEME_STORAGE_KEY = "pathmux.theme-scheme";

export const THEME_DEFINITIONS: readonly ThemeDefinition[] = [
  { value: "ocean", name: "海湾蓝", description: "清澈现代，专注高效", category: "daily", colors: ["#071d31", "#123a60", "#2496f3", "#65c3ff", "#ef5b63"] },
  { value: "glacier", name: "冰川", description: "清透冷静，轻盈通透", category: "daily", colors: ["#092735", "#17465a", "#55c8e8", "#a7e8f4", "#ef5b63"] },
  { value: "graphite", name: "石墨", description: "经典中性，历久弥新", category: "daily", colors: ["#171a1f", "#2b3038", "#a7b0bd", "#d8dee7", "#ef5b63"] },
  { value: "pine", name: "松林", description: "沉静自然，舒适护眼", category: "daily", colors: ["#092821", "#16473a", "#28b887", "#68d5ac", "#ef5b63"] },
  { value: "sunset", name: "夕照", description: "温暖柔和，激发创造", category: "daily", colors: ["#2b1c18", "#5b3527", "#f28a4b", "#f7bb83", "#ef5b63"] },
  { value: "iris", name: "鸢尾", description: "优雅灵动，富有想象", category: "daily", colors: ["#1d1838", "#39306d", "#8b6cf2", "#b5a1ff", "#ef5b63"] },
  { value: "sakura", name: "樱雾", description: "柔和治愈，轻盈雅致", category: "daily", colors: ["#311e2c", "#63415b", "#e986b8", "#f6bdd9", "#ef5b63"] },
  { value: "apple", name: "苹果", description: "清晰克制，液态玻璃", category: "daily", colors: ["#111318", "#2c2c2e", "#0a84ff", "#d9efff", "#ff453a"] },
  { value: "sky", name: "晴空", description: "澄澈明亮，自在舒展", category: "daily", colors: ["#dff4ff", "#f7fcff", "#3ba7e8", "#ffffff", "#ef5b63"] },
  { value: "mint", name: "薄荷", description: "清新自然，轻松醒神", category: "daily", colors: ["#ddf8f1", "#f7fdfb", "#3fc6aa", "#ffffff", "#ef5b63"] },
  { value: "peach", name: "柔光", description: "粉紫晴蓝，柔润轻盈", category: "daily", colors: ["#f8e5ee", "#fbf9ff", "#b7a8d6", "#add9f3", "#ef5b63"] },
  { value: "lavender", name: "云紫", description: "蓝紫弥散，安静通透", category: "daily", colors: ["#dff4ff", "#ffffff", "#5553b8", "#d9d5ff", "#ef5b63"] },
  { value: "spring", name: "新春", description: "梅花灯笼，喜迎新岁", category: "festival", colors: ["#7d1f29", "#bc3441", "#e64b45", "#ffd28a", "#ef5b63"] },
  { value: "lantern", name: "元宵", description: "灯海暖金，欢聚团圆", category: "festival", colors: ["#7a2c21", "#cc5038", "#f06c42", "#ffdda3", "#ef5b63"] },
  { value: "dragonboat", name: "端午", description: "晴空碧水，龙舟竞渡", category: "festival", colors: ["#0f5860", "#228c82", "#2db69a", "#a6eadb", "#ef5b63"] },
  { value: "midautumn", name: "中秋", description: "明月桂香，温暖团圆", category: "festival", colors: ["#314878", "#5c70bd", "#788eea", "#ffe4a3", "#ef5b63"] },
  { value: "national", name: "国庆", description: "朝阳红绸，山河明朗", category: "festival", colors: ["#7b2027", "#b7333a", "#e94443", "#ffd07a", "#ef5b63"] },
] as const;

export const DEFAULT_SCHEME: Scheme = "ocean";

const VALID_SCHEMES = new Set<Scheme>(THEME_DEFINITIONS.map((theme) => theme.value));

type ThemeStorage = Pick<Storage, "getItem" | "setItem">;

export function isScheme(value: string | null): value is Scheme {
  return value !== null && VALID_SCHEMES.has(value as Scheme);
}

export function readStoredScheme(storage: ThemeStorage): Scheme {
  try {
    const stored = storage.getItem(THEME_SCHEME_STORAGE_KEY);
    // 兼容 3.0.10 及更早版本的 a / b 两套强调色。
    if (stored === "a") return "sunset";
    if (stored === "b") return "ocean";
    // “暖砂”已升级为苹果风格，保留老用户的选择位置。
    if (stored === "sand") return "apple";
    return isScheme(stored) ? stored : DEFAULT_SCHEME;
  } catch {
    return DEFAULT_SCHEME;
  }
}

const LUNAR_FESTIVALS = new Map<string, Scheme>([
  ["1-1", "spring"],
  ["1-15", "lantern"],
  ["5-5", "dragonboat"],
  ["8-15", "midautumn"],
]);

/** 返回日期对应的中国节日主题；国庆按公历，其余按中国农历。 */
export function festivalSchemeForDate(date: Date): Scheme | null {
  if (date.getMonth() === 9 && date.getDate() === 1) return "national";

  try {
    const parts = new Intl.DateTimeFormat("zh-CN-u-ca-chinese", {
      month: "numeric",
      day: "numeric",
    }).formatToParts(date);
    const month = Number(parts.find((part) => part.type === "month")?.value);
    const day = Number(parts.find((part) => part.type === "day")?.value);
    return LUNAR_FESTIVALS.get(`${month}-${day}`) ?? null;
  } catch {
    // 极少数缺少 Chinese Calendar 的运行环境仍可正常使用手动主题。
    return null;
  }
}

/** 节日当天优先展示节日主题，但不覆盖用户持久化的日常偏好。 */
export function readInitialScheme(storage: ThemeStorage, date = new Date()): Scheme {
  return festivalSchemeForDate(date) ?? readStoredScheme(storage);
}

export function storeScheme(storage: ThemeStorage, scheme: Scheme): void {
  try {
    storage.setItem(THEME_SCHEME_STORAGE_KEY, scheme);
  } catch {
    // localStorage 被系统策略禁用时，当前会话仍可正常切换主题。
  }
}
