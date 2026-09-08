import { useEffect, useState } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import PersistentPage, { usePageActivation, usePageActive } from "./PersistentPage";

let renderer: ReactTestRenderer;
beforeEach(() => { vi.useFakeTimers(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); });
afterEach(() => { if (renderer) act(() => renderer.unmount()); vi.useRealTimers(); vi.unstubAllGlobals(); });

it("分批后台挂载，切换保留编辑状态，只在重新进入时更新", async () => {
  const mount = vi.fn(), unmount = vi.fn(), refresh = vi.fn();
  function Page() {
    const active = usePageActive();
    const [draft, setDraft] = useState("original");
    useEffect(() => { mount(); return unmount; }, []);
    usePageActivation(refresh);
    return <input value={draft} data-active={active} onChange={() => setDraft("draft")} />;
  }
  const view = (active: boolean) => <PersistentPage active={active} warmupDelay={700}><Page /></PersistentPage>;
  await act(async () => { renderer = create(view(false)); });
  expect(mount).not.toHaveBeenCalled();
  await act(async () => { vi.advanceTimersByTime(700); });
  expect(mount).toHaveBeenCalledTimes(1);
  expect(renderer.root.findByType("input").props["data-active"]).toBe(false);
  await act(async () => { renderer.update(view(true)); });
  expect(refresh).toHaveBeenCalledTimes(1);
  act(() => renderer.root.findByType("input").props.onChange());
  await act(async () => { renderer.update(view(false)); });
  await act(async () => { renderer.update(view(true)); });
  await act(async () => { renderer.update(view(true)); });
  expect(renderer.root.findByType("input").props.value).toBe("draft");
  expect(refresh).toHaveBeenCalledTimes(2);
  expect(mount).toHaveBeenCalledTimes(1);
  expect(unmount).not.toHaveBeenCalled();
});

it("预热前点击立即挂载，卸载后取消预热", async () => {
  await act(async () => { renderer = create(<PersistentPage active warmupDelay={3000}><span>ready</span></PersistentPage>); });
  expect(renderer.root.findByType("span").children).toEqual(["ready"]);
  act(() => renderer.unmount());
  expect(vi.getTimerCount()).toBe(0);
});
