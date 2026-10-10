import { lazy, Suspense } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { expect, it, vi } from "vitest";
import PageErrorBoundary from "./PageErrorBoundary";

it.each([
  "Failed to fetch dynamically imported module", // Windows WebView2
  "Importing a module script failed", // macOS WebKit
  "error loading dynamically imported module",
])("页面文件加载失败（%s）后，重试清除浏览器和 React 的失败缓存", async message => {
  const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const reload = vi.fn();
  vi.stubGlobal("window", { location: { reload } });
  const load = vi.fn()
    .mockRejectedValueOnce(new TypeError(message))
    .mockResolvedValue({ default: () => <p>ChatGPT 多开页面正常</p> });
  const Page = lazy(load);
  let renderer!: ReactTestRenderer;
  try {
    await act(async () => { renderer = create(<PageErrorBoundary><Suspense fallback="加载中"><Page /></Suspense></PageErrorBoundary>); });
    expect(renderer.root.findByProps({ role: "alert" })).toBeTruthy();
    await act(async () => renderer.root.findByType("button").props.onClick());
    expect(reload).toHaveBeenCalledOnce();
    // 新文档创建新的 lazy 实例，模拟浏览器重载后重新请求模块。
    act(() => renderer.unmount());
    const ReloadedPage = lazy(load);
    await act(async () => { renderer = create(<PageErrorBoundary><Suspense fallback="加载中"><ReloadedPage /></Suspense></PageErrorBoundary>); });
    expect(load).toHaveBeenCalledTimes(2);
    expect(renderer.root.findByType("p").children).toEqual(["ChatGPT 多开页面正常"]);
  } finally {
    if (renderer) act(() => renderer.unmount());
    consoleError.mockRestore();
    vi.unstubAllGlobals();
  }
});
