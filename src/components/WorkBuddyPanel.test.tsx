import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Button, Checkbox, Modal, NavLink, Text } from "@mantine/core";
import WorkBuddyPanel from "./WorkBuddyPanel";
import { api, type WorkBuddyState, type WorkBuddyCertificateStatus } from "../api";

vi.mock("@mantine/core", () => ({
  ...Object.fromEntries(["Alert", "Badge", "Button", "Card", "Code", "Group", "Modal", "NavLink", "PasswordInput", "SimpleGrid", "Stack", "Text", "TextInput", "Title"].map((name) => [name, name.toLowerCase()])),
  Checkbox: Object.assign((props: Record<string, unknown>) => <input {...props} />, { Group: "fieldset" }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../api", () => ({ api: { workBuddyState: vi.fn(), listWorkBuddyOrganizationModels: vi.fn(), checkWorkBuddyCertificate: vi.fn() } }));
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}
const state: WorkBuddyState = {
  environment: {
    found: true,
    platform: "windows",
    configPath: "test",
    configExists: true,
    configValid: true,
    detail: "test",
    platformUi: {
      executablePickerTitle: "选择 WorkBuddy.exe",
      executableFilterName: "WorkBuddy 应用程序",
      executableExtensions: ["exe"],
      caImportConsequences: ["Windows test copy"],
    },
  },
  gateway: { url: "https://a.example.test", hasApiKey: true },
  organizations: ["a", "b"].map((id) => ({ id, name: id, modelPrefix: "", url: `https://${id}.example.test`, selectedModels: [`${id}-model`], hasApiKey: true })),
  models: [], revision: "test", gatewayRevision: "test", organizationsRevision: "test", warnings: [],
};
let renderer: ReactTestRenderer;
beforeEach(() => { vi.resetAllMocks(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.mocked(api.workBuddyState).mockResolvedValue(state); });
afterEach(() => { if (renderer) act(() => renderer.unmount()); vi.unstubAllGlobals(); });

// 只取"模型多选"那组勾选框。不要用全局 findAllByType(Checkbox)：
// 面板里还有别的 Checkbox（如风险确认弹窗的「我已了解」），全局查询会把它们一起算进来。
const modelCheckboxValues = () =>
  renderer.root
    .findAllByType(Checkbox.Group)[0]
    .findAllByType(Checkbox)
    .map((item) => item.props.value);

it("再次显示时静默更新数据且保留未保存勾选，手动刷新仍请求网关", async () => {
  vi.mocked(api.listWorkBuddyOrganizationModels).mockResolvedValue(["a-model", "extra-model"]);
  vi.mocked(api.checkWorkBuddyCertificate).mockResolvedValue({ state: "notRequired", detail: "test" });
  await act(async () => { renderer = create(<WorkBuddyPanel active />); });
  act(() => renderer.root.findByType(Checkbox.Group).props.onChange(["extra-model"]));
  await act(async () => { renderer.update(<WorkBuddyPanel active={false} />); });
  expect(renderer.root.findAllByType(Modal).every((modal) => !modal.props.opened)).toBe(true);
  await act(async () => { renderer.update(<WorkBuddyPanel active />); });
  expect(renderer.root.findByType(Checkbox.Group).props.value).toEqual(["extra-model"]);
  expect(api.workBuddyState).toHaveBeenCalledTimes(2);
  expect(api.checkWorkBuddyCertificate).toHaveBeenCalledTimes(2);
  expect(api.listWorkBuddyOrganizationModels).toHaveBeenCalledTimes(2);
  await act(async () => {
    renderer.root.findAllByType(Button).find((button) => button.children.includes("刷新模型"))!.props.onClick();
  });
  expect(api.listWorkBuddyOrganizationModels).toHaveBeenCalledTimes(3);
});

it.each(["resolve", "reject"] as const)("切换组织后忽略旧模型与证书的 %s", async (outcome) => {
  const a = deferred<string[]>(), b = deferred<string[]>();
  const ca = deferred<WorkBuddyCertificateStatus>(), cb = deferred<WorkBuddyCertificateStatus>();
  vi.mocked(api.listWorkBuddyOrganizationModels).mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise);
  vi.mocked(api.checkWorkBuddyCertificate).mockReturnValueOnce(ca.promise).mockReturnValueOnce(cb.promise);
  await act(async () => { renderer = create(<WorkBuddyPanel />); });
  act(() => renderer.root.findAllByType(NavLink)[1].props.onClick());
  await act(async () => {
    if (outcome === "resolve") { a.resolve(["a-model"]); ca.resolve({ state: "untrusted", detail: "stale-certificate" }); }
    else { a.reject(new Error("stale-model")); ca.reject(new Error("stale-certificate")); }
  });
  const refresh = () => renderer.root.findAllByType(Button).find((item) => item.children.includes("刷新模型"))!;
  expect(refresh().props.loading).toBe(true);
  expect(renderer.root.findByType(Checkbox.Group).props.value).toEqual(["b-model"]);
  await act(async () => { b.resolve(["b-model"]); cb.resolve({ state: "notRequired", detail: "current-certificate" }); });
  expect(refresh().props.loading).toBe(false);
  expect(renderer.root.findByType(Checkbox.Group).props.value).toEqual(["b-model"]);
  const content = renderer.root.findAllByType(Text).flatMap((item) => item.children).filter((item) => typeof item === "string").join(" ");
  expect(content).not.toContain("stale-certificate");
  expect(modelCheckboxValues()).toEqual(["b-model"]);
});

it("后台更新慢或失败时保留内容，快速切回合并请求", async () => {
  vi.mocked(api.listWorkBuddyOrganizationModels).mockResolvedValue(["a-model", "extra-model"]);
  vi.mocked(api.checkWorkBuddyCertificate).mockResolvedValue({ state: "notRequired", detail: "test" });
  await act(async () => { renderer = create(<WorkBuddyPanel active />); });
  act(() => renderer.root.findByType(Checkbox.Group).props.onChange(["extra-model"]));
  const pending = deferred<string[]>();
  vi.mocked(api.listWorkBuddyOrganizationModels).mockReturnValueOnce(pending.promise);
  await act(async () => { renderer.update(<WorkBuddyPanel active={false} />); });
  await act(async () => { renderer.update(<WorkBuddyPanel active />); });
  expect(modelCheckboxValues()).toEqual(["a-model", "extra-model"]);
  expect(renderer.root.findAllByType(Button).find((item) => item.children.includes("刷新模型"))!.props.loading).toBe(false);
  await act(async () => { renderer.update(<WorkBuddyPanel active={false} />); });
  await act(async () => { renderer.update(<WorkBuddyPanel active />); });
  expect(api.workBuddyState).toHaveBeenCalledTimes(2);
  await act(async () => { pending.reject(new Error("timeout")); });
  expect(modelCheckboxValues()).toEqual(["a-model", "extra-model"]);
  expect(renderer.root.findByType(Checkbox.Group).props.value).toEqual(["extra-model"]);
});
