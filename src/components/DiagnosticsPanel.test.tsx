import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Text, Alert, Button } from "@mantine/core";
import DiagnosticsPanel from "./DiagnosticsPanel";
import PersistentPage from "./PersistentPage";
import StableRefreshButton from "./StableRefreshButton";
import { api } from "../api";

vi.mock("@mantine/core", () => Object.fromEntries([
  "Alert", "Badge", "Button", "Card", "Code", "Divider", "Group", "Loader", "Stack", "Text", "ThemeIcon", "Title",
].map((name) => [name, name.toLowerCase()])));
// 环境证明卡会调这三个；mock 必须覆盖组件真正用到的全部 API，
// 否则它们 undefined，组件里的 useEffect 会同步抛错。
vi.mock("../api", () => ({
  api: {
    healthCheck: vi.fn(),
    recentSyncLog: vi.fn(),
    syncAll: vi.fn(),
    listProfiles: vi.fn().mockResolvedValue([]),
    profileRuntimeInfo: vi.fn().mockResolvedValue([]),
    lastVerification: vi.fn().mockResolvedValue(null),
  },
}));
let renderer: ReactTestRenderer;
beforeEach(() => {
  vi.resetAllMocks();
  // resetAllMocks 会把工厂里的 mockResolvedValue 一起清掉 —— 必须在这里重设，
  // 否则环境证明卡拿到的会是 undefined，.then 直接抛。
  vi.mocked(api.listProfiles).mockResolvedValue([]);
  vi.mocked(api.profileRuntimeInfo).mockResolvedValue([]);
  vi.mocked(api.lastVerification).mockResolvedValue(null);
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
});
afterEach(() => { if (renderer) act(() => renderer.unmount()); vi.useRealTimers(); vi.unstubAllGlobals(); });
const text = () => renderer.root.findAllByType(Text).flatMap((item) => item.children).filter((item) => typeof item === "string").join(" ");
const mount = async () => { await act(async () => { renderer = create(<DiagnosticsPanel />); }); };
const diagnose = async () => {
  await act(async () => {
    renderer.root.findByType(StableRefreshButton).props.onClick();
    await Promise.resolve();
  });
};

it("首次健康检查失败不能显示健康结论", async () => {
  vi.mocked(api.healthCheck).mockRejectedValue(new Error("网络失败"));
  vi.mocked(api.recentSyncLog).mockResolvedValue([]);
  await mount();
  await diagnose();
  expect(text()).toContain("检测未完成");
  expect(text()).not.toContain("所有检查均正常");
});

it("日志失败不影响成功的健康结果", async () => {
  vi.mocked(api.healthCheck).mockResolvedValue([{ id: "test", label: "test", status: "ok", detail: "ready" }]);
  vi.mocked(api.recentSyncLog).mockRejectedValue(new Error("日志无法读取"));
  await mount();
  await diagnose();
  expect(text()).toContain("所有检查均正常");
  expect(renderer.root.findAllByType(Alert).flatMap((item) => item.children)).toContain("日志读取失败：");
});

it("空健康结果不能算检查成功", async () => {
  vi.mocked(api.healthCheck).mockResolvedValue([]);
  vi.mocked(api.recentSyncLog).mockResolvedValue([]);
  await mount();
  await diagnose();
  expect(text()).toContain("检测未完成");
});

// Mantine 的 loading 属性会把按钮文字换成纯转圈，用户看不出正在同步
// （真机反馈：以为按钮坏了）。改显式 Loader + 文字切换后锁定该行为。
it("同步期间按钮显示「正在同步」并禁用，完成后恢复", async () => {
  vi.mocked(api.healthCheck).mockResolvedValue([{ id: "t", label: "t", status: "ok", detail: "d" }]);
  vi.mocked(api.recentSyncLog).mockResolvedValue([]);
  let release!: (value: string) => void;
  vi.mocked(api.syncAll).mockReturnValue(new Promise<string>((res) => { release = res; }));
  await mount();
  const syncButton = () =>
    renderer.root.findAllByType(Button).find((b) => b.children.join("").includes("同步"))!;
  await act(async () => { syncButton().props.onClick(); await Promise.resolve(); });
  expect(syncButton().props.disabled).toBe(true);
  expect(syncButton().children.join("")).toContain("正在同步");
  await act(async () => { release("done"); });
  expect(syncButton().props.disabled).toBeFalsy();
  expect(syncButton().children.join("")).toContain("同步并修复");
});

it("进入诊断页只展示历史结论，只有点击重新检测才读取钥匙串", async () => {
  vi.useFakeTimers();
  vi.mocked(api.healthCheck).mockResolvedValue([{ id: "test", label: "test", status: "ok", detail: "ready" }]);
  vi.mocked(api.recentSyncLog).mockResolvedValue([]);
  vi.mocked(api.lastVerification).mockResolvedValue({
    at: Math.floor(Date.now() / 1000),
    problems: 0,
    gatewayFails: [],
    appVersion: "3.0.9",
  });
  const view = (active: boolean) => (
    <PersistentPage active={active} warmupDelay={100}>
      <DiagnosticsPanel />
    </PersistentPage>
  );

  await act(async () => { renderer = create(view(false)); });
  await act(async () => { vi.advanceTimersByTime(100); });
  expect(api.healthCheck).not.toHaveBeenCalled();

  await act(async () => { renderer.update(view(true)); });
  expect(api.healthCheck).not.toHaveBeenCalled();
  expect(text()).toContain("最近一次诊断正常");
  expect(text()).not.toContain("尚未诊断");

  await diagnose();
  expect(api.healthCheck).toHaveBeenCalledTimes(1);
});
