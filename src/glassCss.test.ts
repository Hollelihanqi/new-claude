import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

// glass.css 是纯 CSS，Windows / macOS WebView 同一套代码同时生效，
// 断言直接针对样式表文本（与 titleBarLayout.test.ts 的 CSS 防漂移测试同一模式），
// 两个平台在任一开发平台即可同时验证。
const css = readFileSync(resolve(process.cwd(), "src/glass.css"), "utf8");

/** 取「选择器 → 声明块」的声明部分；选择器须紧邻 `{`（多选择器规则的最后一段）。 */
function rule(selector: string): string {
  const esc = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const matches = [...css.matchAll(new RegExp(`${esc}\\s*\\{([^}]+)\\}`, "g"))];
  expect(matches.length, `缺少 ${selector} 样式`).toBeGreaterThan(0);
  return matches.map((m) => m[1]).join("\n");
}

/** 全部深色覆盖规则（html[data-mantine-color-scheme="dark"] 前缀，选择器可跨行含逗号）。 */
function darkRules(): string[] {
  return [
    ...css.matchAll(/html\[data-mantine-color-scheme="dark"\][^{}]*?\{[^}]*\}/g),
  ].map((m) => m[0]);
}

describe("MCP 服务页滚动兜底（与扩展页同一 view-scroll 方案）", () => {
  it("页面根部挂 view-scroll：滚动由 view-scroll 承担，页面自身不再 overflow", () => {
    const panel = readFileSync(
      resolve(process.cwd(), "src/components/mcp/McpPanel.tsx"),
      "utf8"
    );
    expect(panel).toContain('className="view-scroll mcp-page"');
    // 与扩展页同一模式（ExtensionsPanel 的 view-scroll extensions-scroll），不是自创滚动容器
    const extensions = readFileSync(
      resolve(process.cwd(), "src/components/ExtensionsPanel.tsx"),
      "utf8"
    );
    expect(extensions).toContain("view-scroll extensions-scroll");
  });

  it("清理后的状态刷新会排队，不会被正在进行的旧请求静默丢弃", () => {
    const panel = readFileSync(
      resolve(process.cwd(), "src/components/mcp/McpPanel.tsx"),
      "utf8"
    );
    expect(panel).toContain("loadQueue.current");
    expect(panel).toContain("loadQueue.current = task");
    expect(panel).not.toContain("if (inFlight.current) return");
  });

  it("mcp-page 不覆盖 view-scroll 的高度：滚动容器保持确定高度，靠内容溢出驱动", () => {
    const mcp = rule(".mcp-page");
    // height:auto 会让页面高等于内容高、永不溢出，滚动随之失效（真机踩过的回归）
    expect(mcp).not.toMatch(/height\s*:/);
    expect(mcp).toContain("padding-right: 0");
    expect(mcp).not.toMatch(/overflow[dy]?\s*:/); // 滚动交给外层 view-scroll
    // view-scroll 提供确定高度与滚动：height:100% + overflow-y:auto 是整个方案的前提
    const viewScroll = rule(".view-scroll");
    expect(viewScroll).toContain("height: 100%");
    expect(viewScroll).toContain("overflow-y: auto");
  });

  it("空状态表格卡保持不可收缩的 min-height（触发整页滚动的条件本身不变）", () => {
    expect(rule('.mcp-table-card[data-empty="true"]')).toContain("min-height: 300px");
    expect(rule(".view-scroll")).toContain("overflow-y: auto");
  });

  it("矮窗口三处回归：问题卡不被压缩、侧边菜单可滚、内层滚动条不露出圆角卡", () => {
    // ① 问题卡（Alert）默认可收缩，矮窗口时文字被裁；禁止收缩后由整页滚动兜底
    expect(rule(".mcp-page > .mantine-Alert-root")).toContain("flex: 0 0 auto");
    // ② 菜单溢出时 side-nav 内部滚动，而不是被 app-sidebar 的 overflow:hidden 裁掉
    const nav = rule(".side-nav");
    expect(nav).toContain("min-height: 0");
    expect(nav).toContain("overflow-y: auto");
    // ③ 全局 6px 灰滚动条在内层滚动容器上会被当成"灰色直角"，统一隐藏；
    //    交汇角（scrollbar-corner）不随滚动条隐藏，需单独置透明
    expect(css).toContain(".side-nav::-webkit-scrollbar");
    expect(css).toContain(".mcp-table-scroll::-webkit-scrollbar");
    expect(css).toContain(".mcp-table-scroll::-webkit-scrollbar-corner");
    // ④ 空状态行不是数据行，hover 高亮会误导（MCP 与扩展页同源问题）
    expect(css).toMatch(
      /\.mcp-table-card\[data-empty="true"\] \.mantine-Table-tr:hover[\s\S]*background: transparent !important/
    );
    expect(css).toMatch(
      /\.extension-table-card \.mantine-Table-tr:has\(\.extension-empty-state\):hover[\s\S]*background: transparent !important/
    );
  });

  it("滚动条交汇角全局透明：未覆盖的滚动容器（如 instances-card）不再露出系统灰直角", () => {
    expect(css).toContain("*::-webkit-scrollbar-corner { background: transparent; }");
  });
});

describe("深色模式：选中态不得复用浅色第 0 阶（近乎白色）", () => {
  it("环境列表与 WorkBuddy 列表的 NavLink 选中态都有深色覆盖", () => {
    for (const sel of [
      ".instances-card .mantine-NavLink-root[data-active]",
      ".workbuddy-list-card .mantine-NavLink-root[data-active]",
    ]) {
      const dark = darkRules().filter((r) => r.includes(sel));
      expect(dark.length, `${sel} 缺少深色覆盖`).toBeGreaterThan(0);
      for (const r of dark) {
        expect(r).not.toContain("var(--mantine-primary-color-0)");
        expect(r).toContain("color-mix");
        expect(r).toContain("var(--mantine-primary-color-8)");
        expect(r).toContain("var(--mantine-primary-color-3)");
      }
    }
  });

  it("WorkBuddy 模型勾选卡选中态有深色覆盖", () => {
    const dark = darkRules().filter((r) =>
      r.includes(".workbuddy-model-checklist .mantine-Card-root:has(input:checked)")
    );
    expect(dark.length).toBeGreaterThan(0);
    for (const r of dark) {
      expect(r).not.toContain("var(--mantine-primary-color-0)");
      expect(r).toContain("color-mix");
    }
  });

  it("WorkBuddy 右侧编辑区深色下与其他卡片同源（纯 --app-surface，不再混品牌深棕）", () => {
    const dark = darkRules().filter((r) =>
      r.includes(".workbuddy-organization-summary")
    );
    expect(dark.length).toBeGreaterThan(0);
    for (const r of dark) {
      expect(r).toContain("background: var(--app-surface)");
      expect(r).not.toContain("primary-color-9");
    }
  });

  it("空状态图标底不再用品牌第 9 阶（A 组橘橙下呈暗棕橙方块）", () => {
    const dark = darkRules().filter((r) => r.includes(".extension-empty-icon"));
    expect(dark.length).toBeGreaterThan(0);
    for (const r of dark) {
      expect(r).not.toContain("primary-color-9");
      expect(r).toContain("color-mix");
      expect(r).toContain("var(--mantine-primary-color-8)");
    }
  });
});

describe("完整主题系统与提示语义", () => {
  it("每套主题都能驱动背景、侧栏与氛围，而不是只改强调色", () => {
    for (const theme of [
      "glacier", "graphite", "pine", "sunset", "iris", "sakura", "apple",
      "spring", "lantern", "dragonboat", "midautumn", "national",
    ]) {
      const selector = `html[data-theme="${theme}"]`;
      const styles = rule(selector);
      expect(styles).toContain("--theme-base");
      expect(styles).toContain("--theme-deep");
      expect(styles).toContain("--theme-sidebar-start");
      expect(styles).toContain("--theme-sidebar-end");
    }
    expect(rule(".app-sidebar")).toContain("var(--theme-sidebar-start)");
    expect(rule(".app-sidebar")).toContain("var(--theme-sidebar-end)");
  });

  it("设置页包含主题图例、实时预览和四级语义颜色", () => {
    const panel = readFileSync(
      resolve(process.cwd(), "src/components/SettingsPanel.tsx"),
      "utf8"
    );
    expect(panel).toContain("theme-gallery");
    expect(panel).toContain("theme-live-preview");
    expect(panel).toContain("日常风格");
    expect(panel).toContain("节日限定");
    expect(panel).not.toContain("festival-legend");
    expect(rule(".theme-gallery")).toContain("repeat(4");
    expect(css).toContain("--semantic-info");
    expect(css).toContain("--semantic-reminder");
    expect(css).toContain("--semantic-success");
    expect(css).toContain("--semantic-danger");
  });

  it("节日主题使用对应文化图形素材，而不是纯色换肤", () => {
    for (const theme of ["spring", "lantern", "dragonboat", "midautumn", "national"]) {
      expect(rule(`html[data-theme="${theme}"]`)).toContain(`festival-${theme}.png`);
    }
    expect(rule(".app-shell::before")).toContain("var(--theme-decoration)");
    expect(rule(".app-sidebar::before")).toContain("var(--theme-decoration)");
  });

  it("苹果主题使用动态系统色与独立的液态玻璃素材", () => {
    const apple = rule('html[data-theme="apple"]');
    const appleDark = rule('html[data-mantine-color-scheme="dark"][data-theme="apple"]');
    const panel = readFileSync(
      resolve(process.cwd(), "src/components/SettingsPanel.tsx"),
      "utf8"
    );
    expect(apple).toContain("apple-glass.png");
    expect(apple).toContain("--app-bg: #f2f2f7");
    expect(appleDark).toContain("--app-bg: #000000");
    expect(appleDark).toContain("--theme-base: #0a84ff");
    expect(panel).toContain("data-apple");
    expect(panel).toContain("apple-glass.png");
  });
});

describe("状态点呼吸动效", () => {
  it("ok/warn/bad 三态都呼吸，checking 保持脉冲", () => {
    expect(rule(".instance-health-dot.ok")).toContain(
      "animation: health-dot-breathe"
    );
    expect(rule(".instance-health-dot.warn")).toContain(
      "animation: health-dot-breathe"
    );
    expect(rule(".instance-health-dot.bad")).toContain(
      "animation: health-dot-breathe"
    );
    expect(rule(".instance-health-dot.checking")).toContain("health-dot-pulse");
    expect(css).toMatch(/@keyframes health-dot-breathe\b/);
  });

  it("呼吸节奏分层：绿最快、橙次之、红最沉稳（周期递增）", () => {
    const period = (sel: string) =>
      Number(rule(sel).match(/health-dot-breathe ([\d.]+)s/)?.[1] ?? 0);
    expect(period(".instance-health-dot.ok")).toBeGreaterThan(0);
    expect(period(".instance-health-dot.ok")).toBeLessThan(
      period(".instance-health-dot.warn")
    );
    expect(period(".instance-health-dot.warn")).toBeLessThan(
      period(".instance-health-dot.bad")
    );
  });

  it("尊重系统「减弱动态效果」：全部状态点动画被禁用", () => {
    const block = css.match(
      /@media \(prefers-reduced-motion: reduce\)\s*\{([\s\S]*?)\n\}/
    )?.[1];
    expect(block, "缺少 prefers-reduced-motion 媒体查询").toBeDefined();
    expect(block).toContain(".instance-health-dot");
    expect(block).toContain("animation: none");
  });
});
