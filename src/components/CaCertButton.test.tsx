import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import CaCertButton from "./CaCertButton";
import { Button, Select, Text, TextInput } from "@mantine/core";
import { api, type Profile } from "../api";

vi.mock("@mantine/core", () => Object.fromEntries([
  "Loader", "Card", "Stack", "Group", "Button", "TextInput", "PasswordInput", "Select",
  "Text", "Title", "NavLink", "Badge", "Code", "Alert", "Box", "Autocomplete", "Modal",
].map((name) => [name, name.toLowerCase()])));
vi.mock("./PersistentPage", () => ({ usePageActive: () => true }));
vi.mock("./RiskConfirm", () => ({
  default: ({ opened, onConfirm }: { opened: boolean; onConfirm: () => void }) =>
    opened ? <button onClick={onConfirm}>confirm</button> : null,
}));
vi.mock("../api", () => ({ api: {
  listProfiles: vi.fn(), importCertFor: vi.fn(), clearCertsFor: vi.fn(),
} }));

const profile = (name: string, type: Profile["type"]): Profile => ({
  name, type, baseUrl: type === "router" ? `https://${name}.example.test` : "",
  hasToken: true, opusModel: "", sonnetModel: "", haikuModel: "",
});

const textOf = (node: unknown): string => {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (node && typeof node === "object" && "props" in node) {
    return textOf((node as { props: { children?: unknown } }).props.children);
  }
  return "";
};

// 审查第 3 条：CA 必须**按网关隔离**。这个组件是用户唯一能选择"导入给谁"的地方，
// 所以它的目标解析错了，隔离就等于没做 —— 哪怕后端全都对。
describe("CA 证书的目标网关", () => {
  let renderer: ReactTestRenderer;

  const openDialog = async () => {
    const button = renderer.root
      .findAllByType(Button)
      .find((b) => b.children.map(textOf).join("").includes("CA 证书"))!;
    await act(async () => { button.props.onClick(); });
  };
  const select = () => renderer.root.findAllByType(Select)[0];
  const allText = () =>
    renderer.root.findAllByType(Text).map((t) => textOf(t.children)).join(" ");

  beforeEach(async () => {
    vi.resetAllMocks();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    // 只有 router 才是"网关环境"：独立登录环境不注入 CA，不该出现在目标列表里
    vi.mocked(api.listProfiles).mockResolvedValue([
      profile("corp", "router"),
      profile("test", "router"),
      profile("alt", "account"),
    ]);
    vi.mocked(api.importCertFor).mockResolvedValue("已导入");
    vi.mocked(api.clearCertsFor).mockResolvedValue("已清空");
    await act(async () => {
      renderer = create(<CaCertButton env={null} />);
    });
  });
  afterEach(() => { act(() => renderer.unmount()); vi.unstubAllGlobals(); });

  it("默认作用于所有网关环境，且只列出网关（独立登录环境不参与）", async () => {
    await openDialog();
    const data = select().props.data as { value: string; label: string }[];
    expect(data.map((d) => d.value)).toEqual(["__all__", "corp", "test"]);
    expect(data[0].label).toContain("2 个");
    expect(data.map((d) => d.value)).not.toContain("alt");
  });

  it("选中单个网关时只导入给它一个", async () => {
    await openDialog();
    await act(async () => { select().props.onChange("corp"); });
    // 填路径 → 点导入 → 确认
    const input = renderer.root.findAllByType(TextInput)[0];
    await act(async () => { input.props.onChange({ currentTarget: { value: "/tmp/ca.pem" } }); });
    const importBtn = renderer.root
      .findAllByType(Button)
      .find((b) => b.children.map(textOf).join("") === "导入")!;
    await act(async () => { importBtn.props.onClick(); });
    const confirm = renderer.root
      .findAllByType("button")
      .find((b) => textOf(b.props.children) === "confirm")!;
    await act(async () => { confirm.props.onClick(); });
    expect(api.importCertFor).toHaveBeenCalledWith(["corp"], "/tmp/ca.pem");
  });

  it("文案明确说明按网关隔离，且不再宣称整机共享", async () => {
    await openDialog();
    expect(allText()).toContain("各自保存");
    expect(allText()).not.toContain("整机共享");
  });
});
