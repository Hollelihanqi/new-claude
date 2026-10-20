import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ActionIcon, Button, Drawer, Modal, Switch } from "@mantine/core";
import PluginManager from "./PluginManager";
import { api } from "../api";

vi.mock("@mantine/core", () => {
  const out: Record<string, unknown> = Object.fromEntries(
    ["ActionIcon", "Alert", "Badge", "Button", "Checkbox", "Drawer", "Group", "Loader", "Modal", "Select", "Stack", "Switch", "Text", "TextInput", "ThemeIcon"].map(name => [name, name.toLowerCase()]),
  );
  for (const [name, parts] of Object.entries({ Table: ["Thead", "Tbody", "Tr", "Th", "Td"], Menu: ["Target", "Dropdown", "Item"], Tabs: ["List", "Tab", "Panel"] })) {
    out[name] = Object.assign(({ children }: { children?: React.ReactNode }) => children, Object.fromEntries(parts.map(part => [part, `${name}-${part}`])));
  }
  return out;
});
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("./PersistentPage", () => ({ usePageActivation: () => {} }));
vi.mock("../api", () => ({ api: { pluginsOverview: vi.fn(), pluginTargets: vi.fn(), managePlugin: vi.fn() } }));
let renderer: ReactTestRenderer;
beforeEach(async () => {
  vi.resetAllMocks();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(api.pluginTargets).mockResolvedValue(["hq"]);
  vi.mocked(api.pluginsOverview).mockResolvedValue([{ name: "ecc@m", shared: false, defaultClaude: false, envs: [{ env: "hq", installed: true, value: true, inherited: false, reason: "" }] }]);
  vi.mocked(api.managePlugin).mockImplementation(() => new Promise(() => {}));
  await act(async () => { renderer = create(<PluginManager />); });
});
afterEach(() => { act(() => renderer.unmount()); vi.unstubAllGlobals(); });
const update = () => renderer.root.findAllByType(Button).find(node => node.props["aria-label"] === "更新 ecc@m")!;
const more = () => renderer.root.findAllByType(ActionIcon).find(node => node.props["aria-label"] === "ecc 更多操作")!;
it("更新直接执行，不打开弹窗或抽屉，也不改变其他按钮状态", async () => {
  await act(async () => { update().props.onClick(); });
  expect(api.managePlugin).toHaveBeenCalledWith("update", "ecc@m", ["hq"], false);
  expect(renderer.root.findByType(Modal).props.opened).toBe(false);
  expect(renderer.root.findByType(Drawer).props.opened).toBe(false);
  expect(update().props.disabled).toBe(true);
  expect(renderer.root.findByType(Switch).props.disabled).toBe(false);
  expect(more().props.disabled).toBe(false);
});
it("启停只改变开关忙碌状态，更新和更多操作保持原状", async () => {
  await act(async () => { renderer.root.findByType(Switch).props.onChange(); });
  expect(api.managePlugin).toHaveBeenCalledWith("disable", "ecc@m", ["hq"], false);
  expect(renderer.root.findByType(Switch).props.disabled).toBe(true);
  expect(update().props.disabled).toBe(false);
  expect(more().props.disabled).toBe(false);
});
