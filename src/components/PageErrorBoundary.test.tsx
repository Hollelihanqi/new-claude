import { act, create } from "react-test-renderer";
import { expect, it, vi } from "vitest";
import PageErrorBoundary from "./PageErrorBoundary";

it("页面抛错显示恢复入口，重试与切换页面都可恢复", () => {
  const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
  let broken = true;
  function Page() {
    if (broken) throw new Error("render failed");
    return <p>页面正常</p>;
  }
  let renderer!: ReturnType<typeof create>;
  try {
    act(() => { renderer = create(<PageErrorBoundary key="a"><Page /></PageErrorBoundary>); });
    expect(renderer.root.findByProps({ role: "alert" })).toBeTruthy();
    broken = false;
    act(() => renderer.root.findByType("button").props.onClick());
    expect(renderer.root.findByType("p").children).toEqual(["页面正常"]);
    broken = true;
    act(() => renderer.update(<PageErrorBoundary key="a"><Page /></PageErrorBoundary>));
    expect(renderer.root.findByProps({ role: "alert" })).toBeTruthy();
    broken = false;
    act(() => renderer.update(<PageErrorBoundary key="b"><Page /></PageErrorBoundary>));
    expect(renderer.root.findByType("p").children).toEqual(["页面正常"]);
  } finally {
    if (renderer) act(() => renderer.unmount());
    consoleError.mockRestore();
  }
});
