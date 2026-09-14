import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import TitleBar from "./TitleBar";
import { ActionIcon, Tooltip } from "@mantine/core";

vi.mock("@mantine/core", () => Object.fromEntries(
  ["ActionIcon", "Tooltip"].map((name) => [name, name.toLowerCase()]),
));
vi.mock("@tabler/icons-react", () => ({
  IconMinus: "svg", IconSquare: "svg", IconBoxMultiple: "svg", IconX: "svg",
}));

// vi.mock 会被提升到文件顶部，所以共享 spy 必须先经 vi.hoisted 建好，否则工厂里会撞暂时性死区。
const appWindow = vi.hoisted(() => ({
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
  isMaximized: vi.fn(),
  onResized: vi.fn(),
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => appWindow }));

const render = async (platform: string | null) => {
  let renderer!: ReactTestRenderer;
  await act(async () => {
    renderer = create(<TitleBar platform={platform} />);
  });
  return renderer;
};

const labels = (renderer: ReactTestRenderer) =>
  renderer.root.findAllByType(ActionIcon).map((n) => String(n.props["aria-label"]));

describe("自绘标题栏", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    appWindow.isMaximized.mockResolvedValue(false);
    appWindow.onResized.mockResolvedValue(() => {});
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("Windows：三个窗口按钮，点击各自调用对应命令", async () => {
    const renderer = await render("windows");
    expect(labels(renderer)).toEqual(["最小化", "最大化", "关闭"]);

    const buttons = renderer.root.findAllByType(ActionIcon);
    await act(async () => { buttons[0].props.onClick(); });
    await act(async () => { buttons[1].props.onClick(); });
    await act(async () => { buttons[2].props.onClick(); });

    expect(appWindow.minimize).toHaveBeenCalledTimes(1);
    expect(appWindow.toggleMaximize).toHaveBeenCalledTimes(1);
    expect(appWindow.close).toHaveBeenCalledTimes(1);
  });

  // 回归：窗口按钮**不得**挂悬浮提示。原生窗口按钮从不弹气泡，移上去弹「最小化」只会打扰；
  // 这里曾照仓库里「问号按钮」的范式套过 Tooltip，被用户当场指出。
  it("窗口按钮不挂悬浮提示", async () => {
    const renderer = await render("windows");
    expect(renderer.root.findAllByType(Tooltip)).toHaveLength(0);
  });

  it("Windows：标题栏与应用名都带拖动区域属性", async () => {
    const renderer = await render("windows");
    // 该属性只在直接命中的元素上生效，所以应用名那段文字必须**各自**带一份，
    // 否则点文字既不能拖窗口、也不能双击最大化。
    const draggable = (className: string) =>
      renderer.root.findByProps({ className }).props["data-tauri-drag-region"];
    expect(draggable("app-titlebar")).toBe(true);
    expect(draggable("app-titlebar-name")).toBe(true);
  });

  it("macOS：不自绘窗口按钮（有原生红黄绿灯），左侧让出内边距", async () => {
    const renderer = await render("macos");
    expect(labels(renderer)).toEqual([]);
    expect(appWindow.minimize).not.toHaveBeenCalled();

    const bar = renderer.root.findByProps({ className: "app-titlebar" });
    expect(bar.props.style.paddingLeft).toBeGreaterThan(40);
  });

  it("非 Windows/macOS：整体不渲染，避免与原生标题栏形成两条", async () => {
    const renderer = await render("other");
    expect(renderer.toJSON()).toBeNull();
  });

  it("平台未知时仍给出窗口按钮（fail-safe：无边框窗口不能没有关闭入口）", async () => {
    const renderer = await render(null);
    expect(labels(renderer)).toHaveLength(3);
  });

  it("最大化后按钮改为「还原」", async () => {
    appWindow.isMaximized.mockResolvedValue(true);
    const renderer = await render("windows");
    expect(labels(renderer)).toEqual(["最小化", "还原", "关闭"]);
  });
});
