import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 期望前端跑在固定端口
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // 洞察页按需加载；提前预构建图表模块，避免首次打开时优化器刷新整个桌面页面。
  optimizeDeps: {
    include: ["echarts/core", "echarts/charts", "echarts/components", "echarts/renderers"],
  },
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
