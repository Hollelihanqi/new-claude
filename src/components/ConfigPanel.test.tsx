import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ConfigPanel from "./ConfigPanel";
import StableRefreshButton from "./StableRefreshButton";
import { NavLink, Autocomplete, PasswordInput, TextInput, Alert, Button } from "@mantine/core";
import { api, type Profile } from "../api";

vi.mock("@mantine/core", () => Object.fromEntries([
  "Loader", "Card", "Stack", "Group", "Button", "TextInput", "PasswordInput", "Select",
  "Text", "Title", "NavLink", "Badge", "Code", "Alert", "Box", "Autocomplete", "Modal",
].map((name) => [name, name.toLowerCase()])));
vi.mock("./InstanceSettingsCard", () => ({ default: () => null }));
vi.mock("../api", () => ({ api: {
  listProfiles: vi.fn(), modelPinWarnings: vi.fn(), profileRuntimeInfo: vi.fn(),
  detectModelsFor: vi.fn(), detectModels: vi.fn(), deleteProfile: vi.fn(),
} }));

const profile = (name: string): Profile => ({
  name, type: "router", baseUrl: `https://${name}.example.test`, hasToken: true,
  opusModel: "old-model", sonnetModel: "old-thinking", haikuModel: "old-model",
});
const deferred = () => {
  let resolve!: (value: string[]) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<string[]>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
};

describe("空间模型检测", () => {
  let renderer: ReactTestRenderer;
  beforeEach(async () => {
    vi.resetAllMocks();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.mocked(api.listProfiles).mockResolvedValue([profile("a"), profile("b")]);
    vi.mocked(api.modelPinWarnings).mockResolvedValue([]);
    vi.mocked(api.profileRuntimeInfo).mockResolvedValue([]);
    vi.mocked(api.deleteProfile).mockResolvedValue("已彻底删除");
    await act(async () => { renderer = create(<ConfigPanel env={null} usageData={null} />); });
    act(() => renderer.root.findAllByType(NavLink)[0].props.onClick());
  });
  afterEach(() => { act(() => renderer.unmount()); vi.unstubAllGlobals(); });
  const detect = () => renderer.root.findByType(StableRefreshButton);
  const options = () => renderer.root.findAllByType(Autocomplete)[0].props.data;
  const messages = () => renderer.root.findAllByType(Alert).flatMap((alert) => alert.children.filter((child) => typeof child === "string")).join(" ");

  it("移除空间只有彻底删除选项，并调用不可降级的删除接口", async () => {
    act(() => renderer.root.findAllByType(Button).find((button) => button.children.includes("移除"))!.props.onClick());
    const buttons = renderer.root.findAllByType(Button);
    expect(buttons.some((button) => button.children.includes("仅移除，保留历史数据"))).toBe(false);
    const confirm = buttons.find((button) => button.children.includes("确认彻底删除"))!;
    await act(async () => { await confirm.props.onClick(); });
    expect(api.deleteProfile).toHaveBeenCalledWith("a");
    expect(messages()).toContain("已彻底删除");
  });

  it("检测候选去重且不混入旧模型，输入框保留原配置", async () => {
    vi.mocked(api.detectModelsFor).mockResolvedValue(["new", " new "]);
    await act(async () => { await detect().props.onClick(); });
    expect(options()).toEqual(["new"]);
    expect(renderer.root.findAllByType(Autocomplete).map((input) => input.props.value))
      .toEqual(["old-model", "old-thinking", "old-model"]);
    expect(messages()).toContain("检测到 1 个可用模型");
    const feedback = renderer.root.findAllByType(Alert).find((alert) => alert.props["data-model-detection-status"]);
    expect(feedback?.props.role).toBe("status");
    expect(feedback?.children.join("")).toContain("检测到 1 个可用模型");
    act(() => renderer.root.findAllByType(NavLink)[1].props.onClick());
    expect(messages()).not.toContain("检测到");
  });

  it.each(["resolve", "reject"] as const)("切换实例后忽略旧请求的 %s，不能结束新请求", async (outcome) => {
    const first = deferred();
    const second = deferred();
    vi.mocked(api.detectModelsFor).mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    let pending!: Promise<void>;
    act(() => { pending = detect().props.onClick(); });
    act(() => renderer.root.findAllByType(NavLink)[1].props.onClick());
    let current!: Promise<void>;
    act(() => { current = detect().props.onClick(); });
    await act(async () => {
      if (outcome === "resolve") first.resolve(["stale-model"]);
      else first.reject(new Error("stale-error"));
      await pending;
    });
    expect(options()).not.toContain("stale-model");
    expect(messages()).not.toContain("stale-error");
    expect(detect().props.busy).toBe(true);
    await act(async () => { second.resolve(["current-model"]); await current; });
    expect(options()).toContain("current-model");
    expect(detect().props.busy).toBe(false);
  });

  it("已有实例填写新 Key 后检测表单中的连接", async () => {
    act(() => renderer.root.findByType(PasswordInput).props.onChange({ currentTarget: { value: "new-key" } }));
    vi.mocked(api.detectModels).mockResolvedValue(["new-model"]);
    await act(async () => { await detect().props.onClick(); });
    expect(api.detectModels).toHaveBeenCalledWith(profile("a").baseUrl, "new-key");
    expect(api.detectModelsFor).not.toHaveBeenCalled();
  });

  it("修改地址但未填写 Key 时明确要求先保存，不悄悄检测旧地址", async () => {
    act(() => renderer.root.findAllByType(TextInput).find((input) => input.props.value === profile("a").baseUrl)!
      .props.onChange({ currentTarget: { value: "https://changed.example.test" } }));
    await act(async () => { await detect().props.onClick(); });
    expect(api.detectModelsFor).not.toHaveBeenCalled();
    expect(api.detectModels).not.toHaveBeenCalled();
    expect(messages()).toContain("网关地址已修改");
  });

  it("空结果显示错误并允许重试", async () => {
    vi.mocked(api.detectModelsFor).mockResolvedValue([]);
    await act(async () => { await detect().props.onClick(); });
    expect(messages()).toContain("网关未返回可用模型");
    expect(detect().props.busy).toBe(false);
  });

  it("同一检测不重复发送，连接修改后丢弃其结果", async () => {
    const response = deferred();
    vi.mocked(api.detectModelsFor).mockReturnValue(response.promise);
    let pending!: Promise<void>;
    act(() => { pending = detect().props.onClick(); void detect().props.onClick(); });
    expect(api.detectModelsFor).toHaveBeenCalledTimes(1);
    act(() => renderer.root.findByType(PasswordInput).props.onChange({ currentTarget: { value: "changed" } }));
    await act(async () => { response.resolve(["old-connection-model"]); await pending; });
    expect(options()).not.toContain("old-connection-model");
    expect(detect().props.busy).toBe(false);
  });

  it("网络失败后保留表单并可再次检测", async () => {
    vi.mocked(api.detectModelsFor).mockRejectedValueOnce(new Error("连接超时"))
      .mockResolvedValueOnce(["recovered"]);
    await act(async () => { await detect().props.onClick(); });
    expect(messages()).toContain("连接超时");
    expect(renderer.root.findAllByType(Autocomplete)[0].props.value).toBe("old-model");
    await act(async () => { await detect().props.onClick(); });
    expect(options()).toContain("recovered");
  });
});
