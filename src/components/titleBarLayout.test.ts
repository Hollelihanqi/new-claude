import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { TITLEBAR_HEIGHT_PX, titleBarLayout } from "./titleBarLayout";

// 平台分支的判据全部集中在这个纯函数里，所以两个分支都能在**任一开发平台**上断言，
// 不依赖 `process.platform`（CLAUDE.md 的硬要求）。
describe("自绘标题栏的平台差异", () => {
  const cases: Array<[string, boolean, boolean, boolean]> = [
    // platform, render, showWindowControls, showAppName
    ["windows", true, true, true],
    ["macos", true, false, false],
    ["other", false, false, false],
  ];

  it.each(cases)("%s 的渲染决策", (platform, render, controls, name) => {
    const layout = titleBarLayout(platform);
    expect(layout.render).toBe(render);
    expect(layout.showWindowControls).toBe(controls);
    expect(layout.showAppName).toBe(name);
  });

  it("macOS 必须为原生红黄绿灯让出左侧内边距，Windows 不需要", () => {
    const mac = titleBarLayout("macos").leftInsetPx;
    const win = titleBarLayout("windows").leftInsetPx;
    expect(mac).toBeGreaterThan(win);
    // 三个灯加间距，太窄会把内容压在灯下面
    expect(mac).toBeGreaterThanOrEqual(70);
  });

  it("平台未知时按「要画窗口按钮」处理", () => {
    // fail-safe：无边框窗口若没有关闭按钮，用户就只剩 Alt+F4 / 任务栏可用。
    // 反过来 macOS 在 platform 到位前最多闪一下自绘按钮，且有原生红黄绿灯兜底。
    for (const platform of [undefined, null, "", "linux", "freebsd"]) {
      expect(titleBarLayout(platform).showWindowControls).toBe(true);
      expect(titleBarLayout(platform).render).toBe(true);
    }
  });

  it("只有明确是 macOS 时才隐藏自绘按钮", () => {
    expect(titleBarLayout("macos").showWindowControls).toBe(false);
    expect(titleBarLayout("MACOS").showWindowControls).toBe(true); // 大小写不匹配就不按 macOS 处理
  });

  // 防漂移：高度同时存在于 TS 常量与 CSS 变量，两边必须一致，否则改了一边就会错位。
  it("CSS 变量 --app-titlebar-height 与 TITLEBAR_HEIGHT_PX 一致", () => {
    const css = readFileSync(resolve(process.cwd(), "src/glass.css"), "utf8");
    expect(css).toContain(`--app-titlebar-height: ${TITLEBAR_HEIGHT_PX}px;`);
  });
});
