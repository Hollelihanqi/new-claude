import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ExtensionsPanel from "./ExtensionsPanel";
import { Badge, Button, Select, Switch, Text } from "@mantine/core";
import { api, type PluginRow } from "../api";

vi.mock("@mantine/core", () => {
  // ⚠️ vi.mock 的工厂会被**提升**到文件顶部，所以这里不能引用任何顶层变量 ——
  // 表头所需的透传组件必须在工厂内部构造。
  //
  // Mantine 的表头是复合组件（Table.Tr / Table.Td …），mock 时不能只给一个字符串，
  // 否则 Table.Thead 是 undefined、渲染直接崩。用透传组件把子节点原样放行。
  const passthrough = ({ children }: { children?: unknown }) => children ?? null;
  const Table: Record<string, unknown> = passthrough as unknown as Record<string, unknown>;
  for (const sub of ["Thead", "Tbody", "Tr", "Th", "Td"]) Table[sub] = passthrough;

  const names = [
    "Loader", "Card", "Stack", "Group", "Button", "TextInput", "PasswordInput", "Select",
    "Text", "Title", "NavLink", "Badge", "Code", "Alert", "Box", "Autocomplete", "Modal",
    "SimpleGrid", "ThemeIcon", "Switch", "Tooltip",
  ];
  const out: Record<string, unknown> = Object.fromEntries(names.map((n) => [n, n.toLowerCase()]));
  out.Table = Table;
  return out;
});
vi.mock("@mantine/notifications", () => ({ notifications: { show: vi.fn() } }));
vi.mock("./PersistentPage", () => ({ usePageActivation: () => {}, usePageActive: () => true }));
vi.mock("./StableRefreshButton", () => ({ default: () => null }));
vi.mock("../api", () => ({ api: {
  extensionOverview: vi.fn(), pluginsOverview: vi.fn(),
  setSharedPlugin: vi.fn(), setEnvPlugin: vi.fn(), restorePluginInheritance: vi.fn(),
} }));

const rows = (): PluginRow[] => [
  { name: "shared@m", shared: true, defaultClaude: false, envs: [
    { env: "corp", value: true, inherited: true, reason: "" },
  ] },
  { name: "overridden@m", shared: true, defaultClaude: null, envs: [
    { env: "corp", value: false, inherited: false, reason: "已覆盖共享配置（你修改过它）" },
  ] },
];

const textOf = (node: unknown): string => {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (node && typeof node === "object" && "props" in node) {
    return textOf((node as { props: { children?: unknown } }).props.children);
  }
  return "";
};

// 插件共享设置的编辑入口（扩展 → Plugins）：
// 默认编辑「所有环境」的共享状态；切到某个环境后设置独立覆盖并提供「恢复继承」；
// 默认 Claude 只展示、不可编辑。
describe("扩展中心的插件共享设置", () => {
  let renderer: ReactTestRenderer;
  const allText = () => renderer.root.findAllByType(Text).map((t) => textOf(t.children)).join(" ");
  const switches = () => renderer.root.findAllByType(Switch);
  const target = () => renderer.root.findAllByType(Select)[0];

  beforeEach(async () => {
    vi.resetAllMocks();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.mocked(api.extensionOverview).mockResolvedValue([]);
    vi.mocked(api.pluginsOverview).mockResolvedValue(rows());
    vi.mocked(api.setSharedPlugin).mockResolvedValue("已更新");
    vi.mocked(api.setEnvPlugin).mockResolvedValue("已设为独立设置");
    vi.mocked(api.restorePluginInheritance).mockResolvedValue("已恢复继承");
    await act(async () => { renderer = create(<ExtensionsPanel />); });
  });
  afterEach(() => { act(() => renderer.unmount()); vi.unstubAllGlobals(); });

  it("默认编辑共享设置，开关反映共享库的值", async () => {
    const data = target().props.data as { value: string; label: string }[];
    expect(data[0].value).toBe("__all__");
    expect(data[0].label).toContain("所有环境");
    expect(data.some((d) => d.value === "corp")).toBe(true);
    // 两个插件的共享开关都应为开
    expect(switches().map((s) => s.props.checked)).toEqual([true, true]);
  });

  it("切换共享开关会写共享库（= 所有环境）", async () => {
    await act(async () => { switches()[0].props.onChange({ currentTarget: { checked: false } }); });
    expect(api.setSharedPlugin).toHaveBeenCalledWith("shared@m", false);
  });

  it("切到某个环境后显示覆盖标记与「恢复继承」，并且不再直接编辑共享库", async () => {
    await act(async () => { target().props.onChange("corp"); });
    // 覆盖的那条要标出来，并给出恢复入口（标记是 Badge，不在 Text 里）
    const badges = renderer.root.findAllByType(Badge).map((b) => textOf(b.props.children));
    expect(badges.some((t) => t.includes("已覆盖"))).toBe(true);
    const restore = renderer.root
      .findAllByType(Button)
      .find((b) => textOf(b.props.children).includes("恢复继承"));
    expect(restore).toBeTruthy();
    // 环境视角下改开关走的是"独立设置"，不是共享库
    await act(async () => { switches()[0].props.onChange({ currentTarget: { checked: false } }); });
    expect(api.setEnvPlugin).toHaveBeenCalledWith("corp", "shared@m", false);
    expect(api.setSharedPlugin).not.toHaveBeenCalled();
  });

  it("「恢复继承」调用对应的命令", async () => {
    await act(async () => { target().props.onChange("corp"); });
    const restore = renderer.root
      .findAllByType(Button)
      .find((b) => textOf(b.props.children).includes("恢复继承"))!;
    await act(async () => { await restore.props.onClick(); });
    expect(api.restorePluginInheritance).toHaveBeenCalledWith("corp", "overridden@m");
  });

  it("默认 Claude 只展示、不出现可编辑开关（应用对它只读）", async () => {
    // 每行只有「共享」/「该环境」两个开关位；默认 Claude 那一列是只读徽标
    expect(allText()).toContain("默认 Claude");
    expect(allText()).toContain("只展示");
    expect(switches().length).toBeLessThanOrEqual(rows().length * 2);
  });
});
