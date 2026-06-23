import React from "react";
import ReactDOM from "react-dom/client";
import { MantineProvider, createTheme } from "@mantine/core";
import "@mantine/core/styles.css";
import "./glass.css";
import App from "./App.jsx";

const theme = createTheme({
  colors: {
    hermes: [
      "#fff0e5",
      "#ffe0cc",
      "#ffc199",
      "#ffa166",
      "#ff8a3d",
      "#ff7a1f",
      "#ff6600",
      "#e65c00",
      "#cc5200",
      "#b34700",
    ],
  },
  primaryColor: "hermes",
  primaryShade: 6,
  fontFamily:
    '-apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", Roboto, sans-serif',
  defaultRadius: "lg",
  components: {
    Card: { defaultProps: { shadow: "sm", className: "glass-card" } },
  },
});

ReactDOM.createRoot(document.getElementById("root")).render(
  <React.StrictMode>
    <MantineProvider theme={theme} defaultColorScheme="auto">
      <App />
    </MantineProvider>
  </React.StrictMode>
);
