/**
 * 自绘标题栏的平台差异。**刻意做成纯函数**：接收后端给的 platform 字符串，
 * 两个分支因此都能在任一开发平台上被测试（CLAUDE.md 的硬要求）。
 *
 * 平台字符串来自后端 `EnvInfo.platform`（`cfg!(target_os=…)` 映射成
 * `"windows" | "macos" | "other"`），前端不自行探测平台。
 */

/** 标题栏高度（逻辑像素）。CSS 侧对应 `--app-titlebar-height`，两者由测试钉住一致。 */
export const TITLEBAR_HEIGHT_PX = 44;

/**
 * macOS 原生红黄绿灯占用的横向宽度。
 *
 * 红黄绿灯浮在 WebView 之上（`titleBarStyle: "Overlay"`），**不是我们画的**，
 * 所以只能靠内边距把内容推开，否则图标会被压在灯下面。
 */
const MAC_TRAFFIC_LIGHT_INSET_PX = 78;

export type TitleBarLayout = {
  /**
   * 是否渲染自绘标题栏。
   *
   * 只有 Windows 的 `tauri.windows.conf.json` 去掉了原生边框；macOS 用的是 Overlay
   * （原生标题栏变成透明浮层，需要我们自己占用这条高度）。其余平台保留原生边框，
   * 渲染自绘条会变成「两条标题栏」。
   */
  render: boolean;
  /**
   * 是否自绘最小化/最大化/关闭。
   *
   * macOS 保留原生红黄绿灯，自绘会与它重复。
   * **平台未知时按"要画"处理**（fail-safe）：无边框窗口若没有关闭按钮，用户只剩
   * Alt+F4 / 任务栏可用；反过来 macOS 在 platform 到位前最多闪一下自绘按钮，且有原生灯兜底。
   */
  showWindowControls: boolean;
  /** 是否显示应用名文字。macOS 平台惯例不显示标题文字。 */
  showAppName: boolean;
  /** 左侧内边距，给 macOS 红黄绿灯让位。 */
  leftInsetPx: number;
};

export function titleBarLayout(platform: string | null | undefined): TitleBarLayout {
  if (platform === "macos") {
    return {
      render: true,
      showWindowControls: false,
      showAppName: false,
      leftInsetPx: MAC_TRAFFIC_LIGHT_INSET_PX,
    };
  }
  if (platform === "other") {
    // 没有对应的平台窗口配置 ⇒ 原生边框还在，不渲染自绘条
    return {
      render: false,
      showWindowControls: false,
      showAppName: false,
      leftInsetPx: 16,
    };
  }
  // "windows"，以及 platform 尚未就绪（undefined/null）时的兜底
  return {
    render: true,
    showWindowControls: true,
    showAppName: true,
    leftInsetPx: 16,
  };
}
