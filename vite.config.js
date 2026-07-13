import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 期望前端跑在固定端口
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // 别监听 Rust 侧目录：target/ 下 cargo 正在写的文件会让 watcher EBUSY 崩掉
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    target: "es2021",
    outDir: "dist",
  },
});
