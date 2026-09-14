import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ExtensionsPanel from "./ExtensionsPanel";
import { Badge, Button, Select, Switch, Text } from "@mantine/core";
import { open } from "@tauri-apps/plugin-dialog";
import { api, type PluginRow, type ResourceOverview } from "../api";

vi.mock("@mantine/core", () => {
  // ⚠️ vi.mock 的工厂会被**提升**到文件顶部，所以这里不能引用任何顶层变量 ——
  // 表头所需的透传组件必须在工厂内部构造。
  //
  // Mantine 的表头是复合组件（Table.Tr / Table.Td …），mock 时不能只给一个字符串，
  // 否则 Table.Thead 是 undefined、渲染直接崩。用透传组件把子节点原样放行。
  const passthrough = ({ children }: { children?: unknown }) => children ?? null;
  const Table: Record<string, unknown> = passthrough as unknown as Record<string, unknown>;
  for (const sub of ["Thead", "Tbody", "Tr", "Th", "Td"]) Table[sub] = passthrough;
  const Tabs: Record<string, unknown> = passthrough as unknown as Record<string, unknown>;
  for (const sub of ["List", "Tab", "Panel"]) Tabs[sub] = passthrough;

  const names = [
    "Loader", "Card", "Stack", "Group", "Button", "TextInput", "PasswordInput", "Select",
    "Text", "Title", "NavLink", "Badge", "Code", "Alert", "Box", "Autocomplete", "Modal",
    "SimpleGrid", "ThemeIcon", "Switch", "Tooltip", "Drawer", "Divider", "Timeline",
  ];
  const out: Record<string, unknown> = Object.fromEntries(names.map((n) => [n, n.toLowerCase()]));
  out.Table = Table;
  out.Tabs = Tabs;
  return out;
});
vi.mock("@mantine/notifications", () => ({ notifications: { show: vi.fn() } }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("./PersistentPage", () => ({ usePageActivation: () => {}, usePageActive: () => true }));
vi.mock("./StableRefreshButton", () => ({ default: () => null }));
vi.mock("./FeatureHelp", () => ({ default: () => null }));
vi.mock("../api", () => ({ api: {
  resourceOverview: vi.fn(), importDefaultResource: vi.fn(), setResourceExcluded: vi.fn(),
  restoreResourceInheritance: vi.fn(), deleteSharedResource: vi.fn(), syncExtensionResources: vi.fn(), installResourceFromPath: vi.fn(),
  setResourceAutoImport: vi.fn(), pluginsOverview: vi.fn(), pluginTargets: vi.fn(), setPluginExcluded: vi.fn(), managePlugin: vi.fn(),
  restorePluginInheritance: vi.fn(),
} }));

const rows = (): PluginRow[] => [
  { name: "shared@m", shared: true, defaultClaude: false, defaultInstalled: true, defaultVersion: "1.0.0", envs: [
    { env: "corp", value: true, inherited: true, reason: "", installed: true, version: "1.0.0", storageIndependent: true },
  ] },
  { name: "overridden@m", shared: true, defaultClaude: null, envs: [
    { env: "corp", value: false, inherited: false, reason: "已覆盖共享配置（你修改过它）", installed: true, storageIndependent: true },
  ] },
];

const resources = (kind: "skills" | "agents"): ResourceOverview => ({
  kind,
  label: kind === "skills" ? "Skill" : "Agent",
  sharedPath: `C:/shared/${kind}`,
  targets: [{ target: "corp", label: "Claude 环境 corp", state: "target", reason: "C:/corp", issues: [] }],
  items: [],
  autoImportEnabled: true,
  lastAutoImportAdded: 0,
  lastAutoImportSkipped: 0,
  lastAutoImportFailures: [],
});

const textOf = (node: unknown): string => {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (node && typeof node === "object" && "props" in node) {
    return textOf((node as { props: { children?: unknown } }).props.children);
  }
  return "";
};

// 插件管理入口（扩展 → Plugins）：所有改变先调用目标环境的 Claude Code 官方命令；
// PathMux 只在命令成功后记录共享 / 独立策略；默认 Claude 只展示、不可编辑。
describe("扩展中心的插件管理", () => {
  let renderer: ReactTestRenderer;
  const allText = () => renderer.root.findAllByType(Text).map((t) => textOf(t.children)).join(" ");
  const switches = () => renderer.root.findAllByType(Switch);
  const target = () => renderer.root.findAllByType(Select).find((node) =>
    (node.props.data as { value: string }[] | undefined)?.some((item) => item.value === "__all__")
  )!;

  beforeEach(async () => {
    vi.resetAllMocks();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.mocked(api.resourceOverview).mockImplementation(async (kind) => resources(kind));
    vi.mocked(api.pluginsOverview).mockResolvedValue(rows());
    vi.mocked(api.pluginTargets).mockResolvedValue(["corp"]);
    vi.mocked(api.restorePluginInheritance).mockResolvedValue("已恢复继承");
    vi.mocked(api.setPluginExcluded).mockResolvedValue("已排除共享策略");
    vi.mocked(api.setResourceAutoImport).mockResolvedValue("已关闭自动导入");
    vi.mocked(api.installResourceFromPath).mockResolvedValue("已安装");
    vi.mocked(api.managePlugin).mockResolvedValue({
      action: "disable", plugin: "shared@m", reloadHint: "请重新加载", results: [{ env: "corp", ok: true, detail: "完成" }],
    });
    await act(async () => { renderer = create(<ExtensionsPanel />); });
  });
  afterEach(() => { act(() => renderer.unmount()); vi.unstubAllGlobals(); });

  it("默认查看所有环境，并展示真实安装数量", async () => {
    const data = target().props.data as { value: string; label: string }[];
    expect(data[0].value).toBe("__all__");
    expect(data[0].label).toContain("所有环境");
    expect(data.some((d) => d.value === "corp")).toBe(true);
    expect(allText()).toContain("已安装 1/1");
  });

  it("自动导入可以关闭，并保留手动导入入口", async () => {
    const autoImport = switches().find((node) => node.props.label === "自动导入新增项")!;
    await act(async () => { await autoImport.props.onChange({ currentTarget: { checked: false } }); });
    expect(api.setResourceAutoImport).toHaveBeenCalledWith(false);
    expect(renderer.root.findAllByType(Button).some((button) => textOf(button.props.children) === "从默认 Claude 导入新增项")).toBe(true);
  });

  it("可以把本地 Skill 安装到共享库", async () => {
    vi.mocked(open).mockResolvedValue("C:/local/reviewer" as never);
    const install = renderer.root.findAllByType(Button).find((button) => textOf(button.props.children) === "安装到共享库")!;
    await act(async () => { await install.props.onClick(); });
    expect(api.installResourceFromPath).toHaveBeenCalledWith("skills", "C:/local/reviewer", undefined);
  });

  it("默认 Claude 的插件只提供身份，安装仍发往选中的环境", async () => {
    const source = renderer.root.findAllByType(Select).find((node) =>
      (node.props.data as { value: string }[] | undefined)?.some((item) => item.value === "shared@m")
    )!;
    await act(async () => { source.props.onChange("shared@m"); });
    const install = renderer.root.findAllByType(Button).find((button) => textOf(button.props.children) === "安装默认 Claude 的插件")!;
    await act(async () => { await install.props.onClick(); });
    expect(api.managePlugin).toHaveBeenCalledWith("install", "shared@m", ["corp"], true);
  });

  it("只保留 Skills、Plugins 与 Agents，不再展示 Commands", () => {
    expect(allText()).toContain("Skills");
    expect(allText()).toContain("Plugins");
    expect(allText()).toContain("Agents");
    expect(allText()).toContain("Commands 已退出独立管理");
  });

  it("所有环境的停用操作通过 Claude Code 官方插件命令逐环境执行", async () => {
    const disable = renderer.root.findAllByType(Button).find((button) => textOf(button.props.children) === "全部停用")!;
    await act(async () => { await disable.props.onClick(); });
    expect(api.managePlugin).toHaveBeenCalledWith("disable", "shared@m", ["corp"], true);
  });

  it("切到某个环境后显示覆盖标记，并用官方命令启停插件", async () => {
    await act(async () => { target().props.onChange("corp"); });
    // 覆盖的那条要标出来，并给出恢复入口（标记是 Badge，不在 Text 里）
    const badges = renderer.root.findAllByType(Badge).map((b) => textOf(b.props.children));
    expect(badges.some((t) => t.includes("已覆盖"))).toBe(true);
    const restore = renderer.root
      .findAllByType(Button)
      .find((b) => textOf(b.props.children).includes("恢复策略"));
    expect(restore).toBeTruthy();
    const pluginSwitch = switches().find((node) => node.props["aria-label"] === "shared@m 停用")!;
    await act(async () => { pluginSwitch.props.onChange({ currentTarget: { checked: false } }); });
    expect(api.managePlugin).toHaveBeenCalledWith("disable", "shared@m", ["corp"], false);
  });

  it("「恢复继承」调用对应的命令", async () => {
    await act(async () => { target().props.onChange("corp"); });
    const restore = renderer.root
      .findAllByType(Button)
      .find((b) => textOf(b.props.children).includes("恢复策略"))!;
    await act(async () => { await restore.props.onClick(); });
    expect(api.restorePluginInheritance).toHaveBeenCalledWith("corp", "overridden@m");
  });

  it("插件排除与停用是两个独立操作", async () => {
    await act(async () => { target().props.onChange("corp"); });
    const exclude = renderer.root.findAllByType(Button).find((button) => textOf(button.props.children) === "排除共享策略")!;
    await act(async () => { await exclude.props.onClick(); });
    expect(api.setPluginExcluded).toHaveBeenCalledWith("corp", "shared@m", true);
    expect(api.managePlugin).not.toHaveBeenCalledWith("disable", "shared@m", ["corp"], false);
  });

  it("当前环境被删除后自动回到所有环境，避免向失效目标发命令", async () => {
    await act(async () => { target().props.onChange("corp"); });
    vi.mocked(api.pluginTargets).mockResolvedValueOnce([]);
    const pluginSwitch = switches().find((node) => node.props["aria-label"] === "shared@m 停用")!;
    await act(async () => { await pluginSwitch.props.onChange({ currentTarget: { checked: false } }); });
    expect(target().props.value).toBe("__all__");
  });

  it("默认 Claude 只读，不能对它执行插件命令", async () => {
    expect(allText()).toContain("默认 Claude");
    expect(allText()).toContain("只读");
    expect((target().props.data as { value: string }[]).some((item) => item.value === "__main__")).toBe(false);
    expect(switches().length).toBeLessThanOrEqual(rows().length * 2);
  });
});
