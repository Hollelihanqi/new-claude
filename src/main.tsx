import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { MantineProvider, createTheme } from "@mantine/core";
import type { MantineColorsTuple } from "@mantine/core";
import "@mantine/core/styles.css";
import "@mantine/notifications/styles.css";
import { Notifications } from "@mantine/notifications";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./glass.css";
import App from "./App";
import {
  festivalSchemeForDate,
  readInitialScheme,
  readStoredScheme,
  storeScheme,
  type Scheme,
} from "./themeScheme";
import {
  colorSchemeFromMedia,
  SYSTEM_DARK_MODE_QUERY,
  watchSystemColorScheme,
} from "./systemColorScheme";

const THEME_COLORS: Record<Scheme, MantineColorsTuple> = {
  ocean: ["#e9f5ff", "#d3eaff", "#a8d5ff", "#78bfff", "#48a9ff", "#2496f3", "#147fd7", "#0b68b5", "#07518e", "#063d6b"],
  glacier: ["#ebfbff", "#d5f5fb", "#aae8f4", "#80daed", "#63cee8", "#55c8e8", "#35afd2", "#228faf", "#196e88", "#124f64"],
  graphite: ["#f1f3f5", "#e3e7eb", "#c8ced5", "#adb5be", "#929ca7", "#76828f", "#606b77", "#4a535d", "#353c44", "#23282e"],
  pine: ["#e8f8f2", "#d1f0e4", "#a8e1cd", "#7bd0b3", "#50c097", "#2eb184", "#20966f", "#167a5b", "#0e5f47", "#084636"],
  sunset: ["#fff2e9", "#ffdfca", "#ffc6a3", "#f9ab79", "#f28f54", "#ec7940", "#d9632e", "#b94d22", "#913a1a", "#6d2b14"],
  iris: ["#f2efff", "#e4ddff", "#cbbdff", "#b19cff", "#9a82fa", "#876df1", "#7456df", "#6041c2", "#4b3299", "#382672"],
  sakura: ["#fff0f7", "#ffe0ef", "#f9bdd9", "#f19bc6", "#e986b8", "#dc6da7", "#c65391", "#a73f77", "#84315f", "#632548"],
  sand: ["#f8f3e9", "#efe5d2", "#decaa7", "#ceb17f", "#c09b61", "#b18a52", "#987344", "#795b36", "#5c452b", "#423221"],
  spring: ["#fff0f1", "#ffdadd", "#ffb5ba", "#f5868e", "#e85b64", "#d93941", "#bd2931", "#9a2028", "#761820", "#551118"],
  lantern: ["#fff2ec", "#ffded1", "#ffc0aa", "#f99b7c", "#ed7957", "#e25f3f", "#c94b32", "#aa3927", "#842a20", "#621f18"],
  dragonboat: ["#eaf8f3", "#d2efe4", "#a9ddca", "#7fcbb0", "#57b996", "#38a77d", "#278c69", "#1d7156", "#145743", "#0d4032"],
  midautumn: ["#fff8e8", "#faedca", "#f0d996", "#e6c36c", "#ddb35d", "#c89a3f", "#a77b2d", "#825e24", "#60451e", "#443118"],
  national: ["#fff0f2", "#ffd9dd", "#ffb1b9", "#f67e89", "#e95361", "#df3545", "#c22434", "#9d1b2a", "#781520", "#551018"],
};

const FESTIVAL_SCHEMES = new Set<Scheme>([
  "spring",
  "lantern",
  "dragonboat",
  "midautumn",
  "national",
]);

function localDateKey(date: Date): string {
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

function Root() {
  const [scheme, setSchemeState] = useState<Scheme>(() => readInitialScheme(window.localStorage));
  const manualOverrideDate = useRef<string | null>(null);
  const observedDate = useRef(localDateKey(new Date()));
  const [colorScheme, setColorScheme] = useState(() =>
    colorSchemeFromMedia(window.matchMedia(SYSTEM_DARK_MODE_QUERY))
  );
  const setScheme = useCallback((value: Scheme) => {
    manualOverrideDate.current = localDateKey(new Date());
    setSchemeState(value);
    storeScheme(window.localStorage, value);
  }, []);

  useEffect(() => {
    const media = window.matchMedia(SYSTEM_DARK_MODE_QUERY);
    setColorScheme(colorSchemeFromMedia(media));
    return watchSystemColorScheme(media, setColorScheme);
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = scheme;
    const palette = THEME_COLORS[scheme];
    const chromeColor = colorScheme === "dark"
      ? palette[FESTIVAL_SCHEMES.has(scheme) ? 7 : 9]
      : palette[1];
    void getCurrentWindow().setBackgroundColor(chromeColor).catch(() => {
      // 浏览器预览环境没有原生窗口；页面主题本身仍可正常工作。
    });
  }, [colorScheme, scheme]);

  useEffect(() => {
    const syncFestivalTheme = () => {
      const now = new Date();
      const today = localDateKey(now);
      if (observedDate.current !== today) {
        observedDate.current = today;
        manualOverrideDate.current = null;
        setSchemeState(festivalSchemeForDate(now) ?? readStoredScheme(window.localStorage));
        return;
      }
      if (manualOverrideDate.current !== today) {
        const festivalTheme = festivalSchemeForDate(now);
        if (festivalTheme) setSchemeState(festivalTheme);
      }
    };

    syncFestivalTheme();
    const timer = window.setInterval(syncFestivalTheme, 60_000);
    return () => window.clearInterval(timer);
  }, []);

  const theme = useMemo(
    () =>
      createTheme({
        colors: { brand: THEME_COLORS[scheme] },
        primaryColor: "brand",
        primaryShade: 6,
        fontFamily:
          '-apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", Roboto, sans-serif',
        defaultRadius: "lg",
        components: {
          Card: { defaultProps: { className: "glass-card" } },
        },
      }),
    [scheme]
  );

  return (
    <MantineProvider theme={theme} forceColorScheme={colorScheme}>
      <Notifications position="top-right" />
      <App scheme={scheme} setScheme={setScheme} />
    </MantineProvider>
  );
}

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("Missing #root element");
createRoot(rootEl).render(<Root />);
