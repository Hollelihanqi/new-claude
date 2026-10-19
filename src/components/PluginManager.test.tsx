import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Button, Checkbox, Drawer, Menu, Modal, TextInput } from "@mantine/core";
import { open } from "@tauri-apps/plugin-dialog";
import { api, type PluginAction, type PluginRow } from "../api";
import PluginManager from "./PluginManager";

vi.mock("@mantine/core", () => {
  const pass = ({ children }: { children?: unknown }) => children ?? null;
  const compound = (parts: string[]) => Object.assign(pass.bind(null), Object.fromEntries(parts.map((name) => [name, name.toLowerCase()])));
  return {
    ...Object.fromEntries(["ActionIcon", "Alert", "Badge", "Button", "Checkbox", "Drawer", "Group", "Loader", "Modal", "Select", "Stack", "Switch", "Text", "TextInput", "ThemeIcon"].map((name) => [name, name.toLowerCase()])),
    Table: compound(["Thead", "Tbody", "Tr", "Th", "Td"]),
    Tabs: compound(["List", "Tab", "Panel"]),
    Menu: compound(["Target", "Dropdown", "Item"]),
  };
});
vi.mock("@mantine/notifications", () => ({ notifications: { show: vi.fn() } }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("./PersistentPage", () => ({ usePageActivation: () => {} }));
vi.mock("../api", () => ({ api: { pluginsOverview: vi.fn(), pluginTargets: vi.fn(), managePlugin: vi.fn(), installPluginPackage: vi.fn() } }));

function textOf(value: unknown): string {
  if (typeof value === "string" || typeof value === "number") return String(value);
  if (Array.isArray(value)) return value.map(textOf).join("");
  if (value && typeof value === "object" && "children" in value) return textOf(value.children);
  return "";
}

describe("A 方案：单环境插件交互", () => {
  let renderer: ReactTestRenderer;
  let rows: PluginRow[];
  const text = () => textOf(renderer.toJSON());
  const button = (label: string) => {
    const scope = renderer.root.findAllByType(Modal).find((node) => node.props.opened)
      ?? renderer.root.findAllByType(Drawer).find((node) => node.props.opened)
      ?? renderer.root;
    return scope.findAllByType(Button).find((node) => textOf(node.children) === label)!;
  };
  const tab = (label: string) => renderer.root.findAllByType("button").find((node) => node.props.role === "tab" && textOf(node.children) === label)!;
  const click = async (label: string) => { await act(async () => { await button(label).props.onClick(); }); };
  const change = async (label: string, checked: boolean) => {
    const control = renderer.root.findAllByType(Checkbox).find((node) => node.props.label === label)!;
    await act(async () => { control.props.onChange({ currentTarget: { checked } }); });
  };
  const apply = (action: PluginAction, plugin: string, env: string) => {
    let row = rows.find((item) => item.name === plugin);
    if (!row) { row = { name: plugin, envs: [] }; rows.push(row); }
    let state = row.envs.find((item) => item.env === env);
    if (!state) { state = { env, inherited: false, reason: "", installed: false, value: null }; row.envs.push(state); }
    if (action === "install") { state.installed = true; state.value = true; state.version = "1.0.0"; }
    if (action === "enable") state.value = true;
    if (action === "disable") state.value = false;
    if (action === "uninstall") { state.installed = false; state.value = null; }
    if (action === "update") state.version = "1.0.1";
    return { action, plugin, results: [{ env, ok: true, detail: "完成" }], reloadHint: action === "update" ? "重启会话应用新版本" : "新会话生效" };
  };

  beforeEach(async () => {
    vi.resetAllMocks(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    rows = [
      { name: "ecc@market", defaultInstalled: true, envs: [
        { env: "hq", installed: true, value: true, version: "2.2.3", inherited: true, reason: "" },
        { env: "ba", installed: false, value: null, inherited: false, reason: "" },
      ] },
      { name: "rust@market", defaultInstalled: true, defaultVersion: "1.0.0", envs: [
        { env: "hq", installed: false, value: null, inherited: false, reason: "" },
        { env: "ba", installed: true, value: false, version: "1.0.0", inherited: false, reason: "" },
      ] },
    ];
    vi.mocked(api.pluginsOverview).mockImplementation(async () => structuredClone(rows));
    vi.mocked(api.pluginTargets).mockResolvedValue(["hq", "ba", "__main__"]);
    vi.mocked(api.managePlugin).mockImplementation(async (action, plugin, envs) => apply(action, plugin, envs[0]));
    vi.mocked(api.installPluginPackage).mockImplementation(async (_source, envs) => apply("install", "local@market", envs[0]));
    await act(async () => { renderer = create(<PluginManager />); });
  });
  afterEach(() => { act(() => renderer.unmount()); vi.unstubAllGlobals(); });

  it("默认选择有效的单环境，显示本环境分组，不暴露聚合或继承操作", () => {
    expect(tab("hq").props["aria-selected"]).toBe(true);
    expect(text()).toContain("当前管理：hq");
    expect(text()).toContain("已安装"); expect(text()).toContain("可添加");
    expect(tab("全部环境")).toBeUndefined(); expect(tab("__main__")).toBeUndefined();
    expect(text()).not.toContain("恢复为批量设置");
    expect(text()).not.toContain("改为单独管理");
    expect(api.managePlugin).not.toHaveBeenCalled();
  });

  it("切换 ba 后只显示 ba 的开关，启用操作只发往 ba", async () => {
    await act(async () => { tab("ba").props.onClick(); });
    const control = renderer.root.findByProps({ "aria-label": "rust@market 在环境 ba 启用" });
    await act(async () => { control.props.onChange(); });
    expect(api.managePlugin).toHaveBeenCalledWith("enable", "rust@market", ["ba"], false);
    expect(rows[0].envs[0].value).toBe(true);
    expect(text()).toContain("当前管理：ba");
  });

  it("安装面板默认只有当前环境，取消不写入", async () => {
    await click("安装插件");
    expect(text()).toContain("仅安装到 hq，其他环境保持不变");
    await click("取消");
    expect(api.managePlugin).not.toHaveBeenCalled();
    expect(api.installPluginPackage).not.toHaveBeenCalled();
  });

  it("选择已发现插件，安装后关闭启用选项会真实执行停用", async () => {
    await click("安装到 hq");
    await change("安装后启用插件", false);
    await click("安装到 hq");
    expect(api.managePlugin).toHaveBeenNthCalledWith(1, "install", "rust@market", ["hq"], false);
    expect(api.managePlugin).toHaveBeenNthCalledWith(2, "disable", "rust@market", ["hq"], false);
    expect(rows[1].envs[0]).toMatchObject({ installed: true, value: false });
  });

  it("更新先确认，取消不执行，确认后只更新当前环境并保留启停", async () => {
    await click("更新");
    expect(renderer.root.findByType(Modal).props.opened).toBe(true);
    await click("取消"); expect(api.managePlugin).not.toHaveBeenCalled();
    await click("更新"); await click("确认更新");
    expect(api.managePlugin).toHaveBeenCalledWith("update", "ecc@market", ["hq"], false);
    expect(rows[0].envs[0].version).toBe("1.0.1");
    expect(text()).toContain("重启会话应用新版本");
  });

  it("卸载从更多操作进入，确认中明确目标，完成后回到可添加区", async () => {
    const remove = renderer.root.findAllByType(Menu.Item).find((node) => textOf(node.children) === "从 hq 卸载")!;
    await act(async () => { remove.props.onClick(); });
    expect(renderer.root.findByType(Modal).props.title).toBe("从 hq 卸载插件？");
    await click("从 hq 卸载");
    expect(api.managePlugin).toHaveBeenCalledWith("uninstall", "ecc@market", ["hq"], false);
    expect(rows[0].envs[0].installed).toBe(false);
    expect(text()).toContain("重新安装");
  });

  it("跨环境复制先选择并预览目标，默认不选任何其他环境", async () => {
    await click("跨环境管理");
    expect(button("应用到 0 个环境").props.disabled).toBe(true);
    await change("ba", true);
    expect(text()).toContain("未安装 → 已启用");
    expect(api.managePlugin).not.toHaveBeenCalled();
    await click("应用到 1 个环境");
    expect(api.managePlugin).toHaveBeenCalledWith("install", "ecc@market", ["ba"], false);
    expect(api.managePlugin).toHaveBeenCalledWith("enable", "ecc@market", ["ba"], false);
    expect(vi.mocked(api.managePlugin).mock.calls.every((call) => call[2].length === 1 && call[2][0] === "ba" && call[3] === false)).toBe(true);
  });

  it("搜索无结果可恢复，不把筛选结果当作环境已清空", async () => {
    const search = renderer.root.findAllByType(TextInput).find((node) => node.props["aria-label"] === "搜索插件")!;
    await act(async () => { search.props.onChange({ currentTarget: { value: "does-not-exist" } }); });
    expect(text()).toContain("无匹配插件");
    await click("清空搜索");
    expect(text()).toContain("ecc"); expect(text()).toContain("rust");
    expect(api.managePlugin).not.toHaveBeenCalled();
  });

  it.each(["C:\\Users\\hq\\plugin.zip", "/Users/hq/plugin.zip"])("本地包选择在两平台路径下都限定为 hq：%s", async (path) => {
    vi.mocked(open).mockResolvedValue(path as never);
    await click("安装插件");
    const tabs = renderer.root.findAll((node) => node.props.value === "remote" && typeof node.props.onChange === "function");
    await act(async () => { tabs[0].props.onChange("local"); });
    await click("选择本地插件包"); await click("安装到 hq");
    expect(api.installPluginPackage).toHaveBeenCalledWith(path, ["hq"], false);
    expect(api.managePlugin).toHaveBeenCalledWith("enable", "local@market", ["hq"], false);
  });
});
