import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ConfigPanel from "./ConfigPanel";
import StableRefreshButton from "./StableRefreshButton";
import { NavLink, Autocomplete, PasswordInput, TextInput, Alert, Button, Text } from "@mantine/core";
import { api, type Profile } from "../api";

vi.mock("@mantine/core", () => Object.fromEntries([
  "Loader", "Card", "Stack", "Group", "Button", "TextInput", "PasswordInput", "Select",
  "Text", "Title", "NavLink", "Badge", "Code", "Alert", "Box", "Autocomplete", "Modal",
].map((name) => [name, name.toLowerCase()])));
vi.mock("./InstanceSettingsCard", () => ({ default: () => null }));
vi.mock("../api", () => ({ api: {
  listProfiles: vi.fn(), modelPinWarnings: vi.fn(), profileRuntimeInfo: vi.fn(),
  detectModelsFor: vi.fn(), detectModels: vi.fn(), deleteProfile: vi.fn(),
  lastVerification: vi.fn(), probeGateway: vi.fn(),
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

describe("环境模型检测", () => {
  let renderer: ReactTestRenderer;
  beforeEach(async () => {
    vi.resetAllMocks();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.mocked(api.listProfiles).mockResolvedValue([profile("a"), profile("b")]);
    vi.mocked(api.modelPinWarnings).mockResolvedValue([]);
    vi.mocked(api.profileRuntimeInfo).mockResolvedValue([]);
    vi.mocked(api.lastVerification).mockResolvedValue(null);
    vi.mocked(api.deleteProfile).mockResolvedValue("已彻底删除");
    await act(async () => { renderer = create(<ConfigPanel env={null} usageData={null} />); });
    act(() => renderer.root.findAllByType(NavLink)[0].props.onClick());
  });
  afterEach(() => { act(() => renderer.unmount()); vi.unstubAllGlobals(); });
  const detect = () => renderer.root.findByType(StableRefreshButton);
  const options = () => renderer.root.findAllByType(Autocomplete)[0].props.data;
  const messages = () => renderer.root.findAllByType(Alert).flatMap((alert) => alert.children.filter((child) => typeof child === "string")).join(" ");

  it("移除环境只有彻底删除选项，并调用不可降级的删除接口", async () => {
    act(() => renderer.root.findAllByType(Button).find((button) => button.children.includes("移除"))!.props.onClick());
    const buttons = renderer.root.findAllByType(Button);
    expect(buttons.some((button) => button.children.includes("仅移除，保留历史数据"))).toBe(false);
    const confirm = buttons.find((button) => button.children.includes("确认彻底删除"))!;
    await act(async () => { await confirm.props.onClick(); });
    expect(api.deleteProfile).toHaveBeenCalledWith("a");
    expect(messages()).toContain("已彻底删除");
  });

  it("启动同步完成后按修订号重新读取运行状态", async () => {
    const before = vi.mocked(api.profileRuntimeInfo).mock.calls.length;
    await act(async () => {
      renderer.update(
        <ConfigPanel env={null} usageData={null} refreshRevision={1} />
      );
    });
    expect(api.profileRuntimeInfo).toHaveBeenCalledTimes(before + 1);
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

  it.each(["resolve", "reject"] as const)("换选环境后忽略旧请求的 %s，不能结束新请求", async (outcome) => {
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

  it("已有环境填写新 Key 后检测表单中的连接", async () => {
    act(() => renderer.root.findByType(PasswordInput).props.onChange({ currentTarget: { value: "new-key" } }));
    vi.mocked(api.detectModels).mockResolvedValue(["new-model"]);
    await act(async () => { await detect().props.onClick(); });
    expect(api.detectModels).toHaveBeenCalledWith(profile("a").baseUrl, "new-key", "a");
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

// 决策 7.4：默认 Claude 的配置应用**只读**。
// 「只读」不等于"不能检查"——可以发现风险并说明原因，但不能替用户改写。
describe("默认 Claude 的模型钉死只告警、不提供一键修复", () => {
  let renderer: ReactTestRenderer;
  // 必须递归进 React 元素（`<b>`、`<Code>` 都是元素，不是字符串），
  // 否则嵌套文案会被丢掉，断言看着通过、其实什么都没查。
  const textOf = (node: unknown): string => {
    if (typeof node === "string" || typeof node === "number") return String(node);
    if (Array.isArray(node)) return node.map(textOf).join("");
    if (node && typeof node === "object" && "props" in node) {
      return textOf((node as { props: { children?: unknown } }).props.children);
    }
    return "";
  };
  const allText = () => renderer.root.findAllByType(Text).map((t) => textOf(t.children)).join(" ");

  beforeEach(async () => {
    vi.resetAllMocks();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.mocked(api.listProfiles).mockResolvedValue([profile("a")]);
    vi.mocked(api.profileRuntimeInfo).mockResolvedValue([]);
    vi.mocked(api.lastVerification).mockResolvedValue(null);
    vi.mocked(api.modelPinWarnings).mockResolvedValue([
      { profile: "__main__", model: "glm-5.2", settingsPath: "/home/u/.claude/settings.json" },
      { profile: "a", model: "glm-4", settingsPath: "/home/u/.claude-split/a/.claude/settings.json" },
    ] as never);
    await act(async () => { renderer = create(<ConfigPanel env={null} usageData={null} />); });
  });
  afterEach(() => { act(() => renderer.unmount()); vi.unstubAllGlobals(); });

  it("默认 Claude 那条没有恢复按钮，并给出配置文件路径供用户自行处理", () => {
    const labels = renderer.root
      .findAllByType(Button)
      .map((b) => b.children.map(textOf).join(""));
    // 只有环境 a 能一键恢复；默认 Claude 那条绝不能有这个入口
    expect(labels.filter((t) => t.includes("恢复档位选择"))).toHaveLength(1);
    expect(allText()).toContain("应用不会修改此配置");
    expect(allText()).toContain("/home/u/.claude/settings.json");
    // 影响范围要写准：只在"从用户主目录启动"时才会被这份配置覆盖
    expect(allText()).toContain("从用户主目录启动");
  });

  it("两条告警并存时，环境的修复说明仍在（不能为了默认 Claude 把这条删掉）", () => {
    expect(allText()).toContain("恢复后会清除固定型号");
    expect(allText()).toContain("环境 a");
  });
});

// Windows RedirectionGuard 修复后的状态语义：扩展目录迁移没完成不再伪装成
// “配置待完善”——那会误导用户去检查网关地址和 Key（实际两者都正常）。
describe("运行状态按失败原因区分文案", () => {
  let renderer: ReactTestRenderer;
  const mount = async (claudeFound: boolean, sharedDirsOk: boolean, gatewayFails: string[] = []) => {
    vi.resetAllMocks();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.mocked(api.listProfiles).mockResolvedValue([profile("a")]);
    vi.mocked(api.modelPinWarnings).mockResolvedValue([]);
    vi.mocked(api.profileRuntimeInfo).mockResolvedValue([{
      name: "a", configDir: "/x", settingsExists: true, hasProjectData: false,
      lastUsed: null, authenticated: true, sharedDirsOk,
    }] as never);
    vi.mocked(api.lastVerification).mockResolvedValue(
      gatewayFails.length ? { at: Math.floor(Date.now() / 1000), problems: gatewayFails.length, gatewayFails } : null,
    );
    await act(async () => {
      renderer = create(
        <ConfigPanel env={claudeFound ? ({ claude_found: true } as never) : null} usageData={null} />,
      );
    });
    act(() => renderer.root.findAllByType(NavLink)[0].props.onClick());
    await act(async () => {});
  };
  afterEach(() => { act(() => renderer.unmount()); vi.unstubAllGlobals(); });
  const statusText = () =>
    renderer.root.findAllByType("strong").map((node) => node.children.join("")).join(" ");

  it("扩展迁移未完成显示准确原因，不再误报配置问题", async () => {
    await mount(true, false);
    expect(statusText()).toContain("扩展迁移未完成");
    expect(statusText()).not.toContain("配置待完善");
  });

  it("扩展迁移完成且登录就绪才显示“环境正常”", async () => {
    await mount(true, true);
    expect(statusText()).toContain("环境正常");
  });

  it("Claude 未检测到时显示 CLI 未就绪", async () => {
    await mount(false, false);
    expect(statusText()).toContain("Claude CLI 未就绪");
  });

  it("上次诊断网关不通的环境显示异常，并提供单环境复测", async () => {
    await mount(true, true, ["a"]);
    expect(statusText()).toContain("网关未连通");
    // 网关不通 = 环境不可用，用红色与「配置待完善」的橙色区分
    const gatewayStrong = () =>
      renderer.root.findAllByType("strong").find((node) => node.children.join("").includes("网关未连通"));
    expect(gatewayStrong()?.props.className).toContain("status-bad");
    vi.mocked(api.probeGateway).mockResolvedValue("网关连通正常，检测到 3 个可用模型。");
    await act(async () => {
      renderer.root.findAllByType(Button).find((b) => b.children.join("") === "检测")!.props.onClick();
    });
    expect(api.probeGateway).toHaveBeenCalledWith("a");
    expect(statusText()).toContain("环境正常");
    // 复测通过后按钮消失：异常态只在有结论支撑时展示
    expect(renderer.root.findAllByType(Button).some((b) => b.children.join("") === "检测")).toBe(false);
  });

  it("单环境复测失败时如实展示失败原因", async () => {
    await mount(true, true, ["a"]);
    vi.mocked(api.probeGateway).mockRejectedValue(new Error("schannel: TLS 握手失败"));
    await act(async () => {
      renderer.root.findAllByType(Button).find((b) => b.children.join("") === "检测")!.props.onClick();
    });
    expect(statusText()).toContain("网关未连通");
    expect(renderer.root.findAllByType(Alert).some((a) => a.children.join("").includes("TLS 握手失败"))).toBe(true);
  });
});
