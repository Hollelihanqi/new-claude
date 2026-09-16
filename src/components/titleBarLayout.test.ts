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
    ["macos", false, false, false],
    ["other", false, false, false],
  ];

  it.each(cases)("%s 的渲染决策", (platform, render, controls, name) => {
    const layout = titleBarLayout(platform);
    expect(layout.render).toBe(render);
    expect(layout.showWindowControls).toBe(controls);
    expect(layout.showAppName).toBe(name);
  });

  it("macOS 使用原生标题栏，只有 Windows 保留自绘栏", () => {
    expect(titleBarLayout("macos")).toMatchObject({
      render: false,
      showWindowControls: false,
      leftInsetPx: 0,
    });
    expect(titleBarLayout("windows")).toMatchObject({
      render: true,
      showWindowControls: true,
    });
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

  it("macOS 配置使用原生可见标题栏，不再设置 Overlay 灯位", () => {
    const config = JSON.parse(
      readFileSync(resolve(process.cwd(), "src-tauri/tauri.macos.conf.json"), "utf8")
    );
    const window = config.app.windows[0];
    expect(window.titleBarStyle).toBe("Visible");
    expect(window.trafficLightPosition).toBeUndefined();
  });

  it("工作区使用独立圆角模块，编辑标题固定在滚动容器顶端", () => {
    const css = readFileSync(resolve(process.cwd(), "src/glass.css"), "utf8");
    const rule = (selector: string) => {
      const matches = [...css.matchAll(new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*\\{([^}]+)\\}`, "g"))];
      expect(matches.length, `缺少 ${selector} 样式`).toBeGreaterThan(0);
      return matches.map((match) => match[1]).join("\n");
    };

    expect(rule(".app-shell")).toContain("gap: 14px");
    expect(rule(".app-sidebar")).toContain("border-radius: 20px");
    expect(rule(".app-header")).toContain("border-radius: 18px");
    const environmentFrames = rule(".instances-pane, .editor-pane");
    expect(environmentFrames).toContain("border-radius: 18px");
    expect(environmentFrames).toContain("overflow: hidden");
    expect(rule(".editor-scroll")).toContain("overflow-y: auto");
    expect(rule(".workbuddy-grid:not(.workbuddy-grid-empty)")).toContain("overflow: hidden");
    expect(rule(".mcp-table-card")).toContain("overflow: hidden");
    expect(rule(".extension-table-card")).toContain("overflow: hidden");
    expect(rule(".editor-toolbar")).toContain("top: 0");
    expect(rule(".editor-toolbar")).not.toMatch(/top:\s*-\d/);
  });
});
