import { useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { MantineProvider, createTheme } from "@mantine/core";
import type { MantineColorsTuple } from "@mantine/core";
import "@mantine/core/styles.css";
import "@mantine/notifications/styles.css";
import { Notifications } from "@mantine/notifications";
import "./glass.css";
import App from "./App";

type Scheme = "a" | "b";

// A 组：橘橙 #fd752c
const brandA: MantineColorsTuple = [
  "#fff2e9", "#ffdcc6", "#ffc09e", "#ffa274", "#ff8a52",
  "#fe7c3c", "#fd752c", "#e7611f", "#cc5316", "#b3460e",
];
// B 组：孔雀蓝 #0c7e9e
const brandB: MantineColorsTuple = [
  "#e7f3f7", "#c5e3eb", "#9dcfdd", "#71bace", "#4ea9c2",
  "#2c98b5", "#0c7e9e", "#0a7791", "#08607a", "#054d63",
];

function Root() {
  const [scheme, setScheme] = useState<Scheme>("b"); // 默认 B 组

  const theme = useMemo(
    () =>
      createTheme({
        colors: { brand: scheme === "a" ? brandA : brandB },
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
    <MantineProvider theme={theme} defaultColorScheme="light">
      <Notifications position="top-right" />
      <App scheme={scheme} setScheme={setScheme} />
    </MantineProvider>
  );
}

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("Missing #root element");
createRoot(rootEl).render(<Root />);
