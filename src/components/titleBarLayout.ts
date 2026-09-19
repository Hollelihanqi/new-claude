/**
 * 自绘标题栏的平台差异。**刻意做成纯函数**：接收后端给的 platform 字符串，
 * 两个分支因此都能在任一开发平台上被测试（CLAUDE.md 的硬要求）。
 *
 * 平台字符串来自后端 `EnvInfo.platform`（`cfg!(target_os=…)` 映射成
 * `"windows" | "macos" | "other"`），前端不自行探测平台。
 */

/** Windows 自绘标题栏高度（逻辑像素）。CSS 侧对应变量，两者由测试钉住一致。 */
export const TITLEBAR_HEIGHT_PX = 44;

export type TitleBarLayout = {
  /**
   * 是否渲染自绘标题栏。
   *
   * Windows 用它承载窗口按钮；macOS 用一条无按钮的可拖拽区域承接透明标题栏，
   * 让最顶部也能跟随当前主题。其他平台保留原生标题栏，不重复渲染。
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
  /** 是否显示应用名文字。 */
  showAppName: boolean;
  /** 自绘栏左侧内边距；当前只供 Windows 使用。 */
  leftInsetPx: number;
};

export function titleBarLayout(platform: string | null | undefined): TitleBarLayout {
  if (platform === "macos") {
    return {
      render: true,
      showWindowControls: false,
      showAppName: false,
      leftInsetPx: 0,
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
