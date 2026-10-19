import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ExtensionsPanel from "./ExtensionsPanel";
import { Button, Switch, Tabs, Text } from "@mantine/core";
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
vi.mock("./PluginManager", () => ({ default: () => null }));
vi.mock("./StableRefreshButton", () => ({ default: () => null }));
vi.mock("./FeatureHelp", () => ({ default: () => null }));
vi.mock("../api", () => ({ api: {
  resourceOverview: vi.fn(), importDefaultResource: vi.fn(), setResourceExcluded: vi.fn(),
  restoreResourceInheritance: vi.fn(), deleteSharedResource: vi.fn(), syncExtensionResources: vi.fn(), installResourceFromPath: vi.fn(),
  setResourceAutoImport: vi.fn(), pluginsOverview: vi.fn(), pluginTargets: vi.fn(), setPluginExcluded: vi.fn(), managePlugin: vi.fn(),
  restorePluginInheritance: vi.fn(), installPluginPackage: vi.fn(),
} }));

const rows = (): PluginRow[] => [
  { name: "shared@m", shared: false, defaultClaude: false, defaultInstalled: true, defaultVersion: "1.0.0", envs: [
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
  if (node && typeof node === "object" && "children" in node) {
    return textOf((node as { children?: unknown }).children);
  }
  return "";
};

// 插件管理入口（扩展 → Plugins）：所有改变先调用目标环境的 Claude Code 官方命令；
// PathMux 只在命令成功后记录共享 / 独立策略；默认 Claude 只展示、不可编辑。
describe("扩展中心的导航与资源管理", () => {
  let renderer: ReactTestRenderer;
  const allText = () => [
    ...renderer.root.findAllByType(Text).map((node) => textOf(node.children)),
    ...renderer.root.findAllByType(Button).map((node) => textOf(node.children)),
    ...renderer.root.findAllByType(Tabs.Tab).map((node) => textOf(node.children)),
    ...renderer.root.findAllByType("button").map((node) => textOf(node.children)),
  ].join(" ");
  const switches = () => renderer.root.findAllByType(Switch);

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
    vi.mocked(api.installPluginPackage).mockResolvedValue({
      action: "install", plugin: "local@pathmux-import", reloadHint: "请重新加载", results: [{ env: "corp", ok: true, detail: "完成" }],
    });
    await act(async () => { renderer = create(<ExtensionsPanel />); });
  });
  afterEach(() => { act(() => renderer.unmount()); vi.unstubAllGlobals(); });

  it("自动导入可以关闭，并保留手动导入入口", async () => {
    const autoImport = switches().find((node) => node.props["aria-label"] === "自动发现新增 Skill")!;
    await act(async () => { await autoImport.props.onChange({ currentTarget: { checked: false } }); });
    expect(api.setResourceAutoImport).toHaveBeenCalledWith(false);
    expect(renderer.root.findAllByType(Button).some((button) => textOf(button.props.children) === "查找并导入")).toBe(true);
  });

  it("可以把本地 Skill 添加到所有环境", async () => {
    vi.mocked(open).mockResolvedValue("C:/local/reviewer" as never);
    const add = renderer.root.findAllByType(Button).find((button) => textOf(button.props.children) === "添加 Skill")!;
    await act(async () => { add.props.onClick(); });
    const install = renderer.root.findAllByType(Button).find((button) => textOf(button.props.children) === "添加到所有环境")!;
    await act(async () => { await install.props.onClick(); });
    expect(api.installResourceFromPath).toHaveBeenCalledWith("skills", "C:/local/reviewer", undefined);
  });

  it("Skill 列表直接显示环境开关和明确操作，不要求用户理解底层策略", async () => {
    act(() => renderer.unmount());
    vi.mocked(api.resourceOverview).mockImplementation(async (kind) => ({
      ...resources(kind),
      items: kind === "skills" ? [{
        name: "reviewer",
        inShared: true,
        inDefaultClaude: true,
        targets: [{
          target: "corp",
          label: "Claude 环境 corp",
          state: "inherited",
          reason: "由 PathMux 管理并保持更新",
          issues: [],
        }],
      }] : [],
    }));
    await act(async () => { renderer = create(<ExtensionsPanel />); });

    const status = switches().find((node) => node.props["aria-label"] === "reviewer 在 Claude 环境 corp 中停用")!;
    expect(status.props.checked).toBe(true);
    expect(textOf(status.props.label)).toBe("已启用");
    expect(allText()).toContain("更新");
    expect(allText()).toContain("移除");
    expect(allText()).not.toContain("继承");
    expect(allText()).not.toContain("排除");

    await act(async () => { await status.props.onChange(); });
    expect(api.setResourceExcluded).toHaveBeenCalledWith("skills", "corp", "reviewer", true);
  });
});
