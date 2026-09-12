import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Text, Alert } from "@mantine/core";
import DiagnosticsPanel from "./DiagnosticsPanel";
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
afterEach(() => { if (renderer) act(() => renderer.unmount()); vi.unstubAllGlobals(); });
const text = () => renderer.root.findAllByType(Text).flatMap((item) => item.children).filter((item) => typeof item === "string").join(" ");
const mount = async () => { await act(async () => { renderer = create(<DiagnosticsPanel />); }); };

it("首次健康检查失败不能显示健康结论", async () => {
  vi.mocked(api.healthCheck).mockRejectedValue(new Error("网络失败"));
  vi.mocked(api.recentSyncLog).mockResolvedValue([]);
  await mount();
  expect(text()).toContain("检测未完成");
  expect(text()).not.toContain("所有检查均正常");
});

it("日志失败不影响成功的健康结果", async () => {
  vi.mocked(api.healthCheck).mockResolvedValue([{ id: "test", label: "test", status: "ok", detail: "ready" }]);
  vi.mocked(api.recentSyncLog).mockRejectedValue(new Error("日志无法读取"));
  await mount();
  expect(text()).toContain("所有检查均正常");
  expect(renderer.root.findAllByType(Alert).flatMap((item) => item.children)).toContain("日志读取失败：");
});

it("空健康结果不能算检查成功", async () => {
  vi.mocked(api.healthCheck).mockResolvedValue([]);
  vi.mocked(api.recentSyncLog).mockResolvedValue([]);
  await mount();
  expect(text()).toContain("检测未完成");
});
