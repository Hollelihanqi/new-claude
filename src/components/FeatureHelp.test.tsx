import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ActionIcon, Drawer } from "@mantine/core";
import FeatureHelp, { type FeatureHelpContent } from "./FeatureHelp";

vi.mock("@mantine/core", () => {
  const names = [
    "ActionIcon", "Badge", "Box", "Code", "Divider", "Drawer", "Group", "Stack",
    "Text", "ThemeIcon", "Tooltip",
  ];
  const out: Record<string, unknown> = Object.fromEntries(names.map((name) => [name, name.toLowerCase()]));
  const passthrough = ({ children }: { children?: unknown }) => children ?? null;
  const Timeline: Record<string, unknown> = passthrough as unknown as Record<string, unknown>;
  Timeline.Item = passthrough;
  out.Timeline = Timeline;
  return out;
});

vi.mock("@tabler/icons-react", () => ({
  IconCode: "icon-code",
  IconHelpCircle: "icon-help-circle",
  IconRoute: "icon-route",
  IconShieldCheck: "icon-shield-check",
}));

const content: FeatureHelpContent = {
  title: "共享 Skills",
  hint: "了解工作原理",
  purpose: "提供共享能力",
  action: "更新目标",
  impact: "影响受管理环境",
  userAction: "无需额外操作",
  flow: ["读取共享库", "更新目标"],
  principles: [{ title: "本地优先", detail: "目标修改后不覆盖", status: "implemented" }],
  example: "corp 收到更新",
  failure: "失败时保留旧版本",
  codeEntries: ["src-tauri/src/extensions.rs"],
};

describe("功能说明按钮", () => {
  let renderer: ReactTestRenderer;

  beforeEach(() => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    act(() => { renderer = create(<FeatureHelp content={content} />); });
  });

  afterEach(() => {
    act(() => renderer.unmount());
    vi.unstubAllGlobals();
  });

  it("点击圆形问号后打开包含设计原理和代码入口的说明面板", () => {
    const button = renderer.root.findByType(ActionIcon);
    const drawer = () => renderer.root.findByType(Drawer);

    expect(button.props["aria-label"]).toBe("了解共享 Skills的工作原理");
    expect(drawer().props.opened).toBe(false);

    act(() => button.props.onClick());

    expect(drawer().props.opened).toBe(true);
    expect(drawer().props.title).toBe("它是怎么工作的 · 共享 Skills");
    const rendered = JSON.stringify(renderer.toJSON());
    expect(rendered).toContain("设计原理");
    expect(rendered).toContain("src-tauri/src/extensions.rs");
  });
});
