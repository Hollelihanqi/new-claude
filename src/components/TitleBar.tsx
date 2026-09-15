import { ActionIcon } from "@mantine/core";
import { IconBoxMultiple, IconMinus, IconSquare, IconX } from "@tabler/icons-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";
import { titleBarLayout } from "./titleBarLayout";

/**
 * 自绘标题栏。
 *
 * 背景：原先是 Windows 原生标题栏，高度约 32px 且**无法调高**（由系统度量 `SM_CAPTION` 决定，
 * Tauri 也没有对应配置项）。要更高的标题栏只能自己画，于是 Windows 侧关掉原生边框
 * （`tauri.windows.conf.json` 的 `decorations: false`），由本组件补上那条栏与窗口按钮。
 *
 * 平台差异**不在这里判断** —— 全部来自后端给的 platform，经 `titleBarLayout` 纯函数换算，
 * 使两个分支都能在任一开发平台上被测试。macOS 使用完整原生标题栏，本组件不渲染；
 * 红黄绿按钮、深浅外观和全屏收起均交给系统处理。
 *
 * 已知取舍：Windows 上关掉原生边框会失去「悬停最大化按钮弹出的贴靠布局浮层」，
 * 无法用 CSS/JS 补回（Windows 只把浮层给对 `WM_NCHITTEST` 回 `HTMAXBUTTON` 的窗口）。
 * 拖到屏幕边缘贴靠与 Win+方向键仍然可用。
 */
export default function TitleBar({ platform }: { platform?: string | null }) {
  const layout = titleBarLayout(platform);
  const [maximized, setMaximized] = useState(false);

  // 只为了把最大化按钮的图标切成「还原」。窗口尺寸变化即最大化状态可能变化。
  useEffect(() => {
    if (!layout.showWindowControls) return;
    const appWindow = getCurrentWindow();
    let alive = true;
    let unlisten: (() => void) | undefined;
    const sync = () => {
      appWindow
        .isMaximized()
        .then((value) => {
          if (alive) setMaximized(value);
        })
        .catch(() => {});
    };
    sync();
    appWindow
      .onResized(sync)
      .then((fn) => {
        // 组件可能在监听器注册完成前就被卸载
        if (alive) unlisten = fn;
        else fn();
      })
      .catch(() => {});
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [layout.showWindowControls]);

  if (!layout.render) return null;

  const appWindow = getCurrentWindow();

  return (
    <div
      className="app-titlebar"
      data-tauri-drag-region
      data-platform={platform ?? "unknown"}
      style={{ paddingLeft: layout.leftInsetPx }}
    >
      {/* 子元素必须**各自**带 data-tauri-drag-region：该属性只在直接命中的元素上生效，
          不给这个 span 加的话，点应用名既不能拖窗口、也不能双击最大化。 */}
      {layout.showAppName && (
        <span className="app-titlebar-name" data-tauri-drag-region>PathMux</span>
      )}

      {/* 刻意**不挂 Tooltip**：原生窗口按钮从不弹提示，悬浮气泡在这里只会打扰。
          `aria-label` 保留，无障碍与测试都靠它。 */}
      {layout.showWindowControls && (
        <div className="app-titlebar-controls">
          <ActionIcon
            className="app-titlebar-button"
            variant="subtle"
            color="gray"
            aria-label="最小化"
            onClick={() => void appWindow.minimize()}
          >
            <IconMinus size={16} />
          </ActionIcon>

          <ActionIcon
            className="app-titlebar-button"
            variant="subtle"
            color="gray"
            aria-label={maximized ? "还原" : "最大化"}
            onClick={() => void appWindow.toggleMaximize()}
          >
            {maximized ? <IconBoxMultiple size={15} /> : <IconSquare size={15} />}
          </ActionIcon>

          <ActionIcon
            className="app-titlebar-button app-titlebar-close"
            variant="subtle"
            color="gray"
            aria-label="关闭"
            onClick={() => void appWindow.close()}
          >
            <IconX size={16} />
          </ActionIcon>
        </div>
      )}
    </div>
  );
}
