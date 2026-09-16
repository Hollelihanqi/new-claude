import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import McpSourceIssuesCard from "./McpSourceIssuesCard";
import { Button, Text } from "@mantine/core";
import type { McpDeadEntry, McpSourceIssue } from "../../api";

vi.mock("@mantine/core", () => Object.fromEntries([
  "Alert", "Button", "Group", "Stack", "Text",
].map((name) => [name, name.toLowerCase()])));
vi.mock("../RiskConfirm", () => ({
  default: ({
    opened,
    onConfirm,
    onCancel,
  }: {
    opened: boolean;
    onConfirm: () => void;
    onCancel: () => void;
  }) =>
    opened ? (
      <>
        <button onClick={onConfirm}>confirm</button>
        <button onClick={onCancel}>cancel</button>
      </>
    ) : null,
}));

const issue = (detail: string): McpSourceIssue => ({
  sourceId: "user:hq",
  path: "C:/x/.claude.json",
  detail,
});
const dead = (rawPath: string): McpDeadEntry => ({
  sourceId: "user:hq",
  rawPath,
  filePath: "C:/x/.claude.json",
});

const textOf = (node: unknown): string => {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (node && typeof node === "object" && "props" in node) {
    return textOf((node as { props: { children?: unknown } }).props.children);
  }
  return "";
};

describe("McpSourceIssuesCard 配置来源问题卡", () => {
  const makeCleanupMock = () => vi.fn(() => Promise.resolve(true));
  let cleanupMock: ReturnType<typeof makeCleanupMock>;
  let renderer: ReactTestRenderer;

  const mount = async (
    issues: McpSourceIssue[],
    deadEntries: McpDeadEntry[],
    opts?: { busy?: boolean; onCleanup?: () => Promise<boolean> },
  ) => {
    cleanupMock = makeCleanupMock();
    const onCleanup =
      opts?.onCleanup ?? (() => { cleanupMock(); return Promise.resolve(true); });
    await act(async () => {
      renderer = create(
        <McpSourceIssuesCard
          issues={issues}
          deadEntries={deadEntries}
          pageActive={true}
          busy={opts?.busy ?? false}
          onCleanup={onCleanup}
        />,
      );
    });
  };
  const findButton = (needle: string) =>
    renderer.root.findAllByType(Button).find((b) => textOf(b.children).includes(needle));
  const confirmButton = () =>
    renderer.root.findAllByType("button").find((b) => textOf(b.children) === "confirm");
  const cancelButton = () =>
    renderer.root.findAllByType("button").find((b) => textOf(b.children) === "cancel");
  const allText = () =>
    renderer.root.findAllByType(Text).map((t) => textOf(t.children)).join(" ");

  beforeEach(() => {
    vi.resetAllMocks();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  });
  afterEach(() => {
    act(() => renderer?.unmount());
    vi.unstubAllGlobals();
  });

  it("issues 为空时不渲染任何内容", async () => {
    await mount([], []);
    expect(renderer.root.children).toHaveLength(0);
    expect(renderer.toJSON()).toBeNull();
  });

  it("有问题但没有可清理的死条目时，只列问题、不显示清理按钮", async () => {
    await mount([issue("mcpServers[x] 不是对象")], []);
    const text = allText();
    expect(text).toContain("mcpServers[x] 不是对象");
    expect(text).toContain("user:hq");
    expect(text).toContain("C:/x/.claude.json");
    expect(findButton("一键清理")).toBeUndefined();
  });

  it("有死条目时显示带数量的清理按钮", async () => {
    await mount([issue("项目键「E:/gone」无法规范化")], [dead("E:/gone"), dead("E:/gone2")]);
    const button = findButton("一键清理")!;
    expect(button).toBeDefined();
    expect(textOf(button.children)).toContain("2");
  });

  it("点清理按钮打开确认弹窗；取消则关闭且不执行清理", async () => {
    await mount([issue("项目键「E:/gone」无法规范化")], [dead("E:/gone")]);
    await act(async () => { findButton("一键清理")!.props.onClick(); });
    expect(confirmButton()).toBeDefined();
    await act(async () => { cancelButton()!.props.onClick(); });
    expect(confirmButton()).toBeUndefined();
    expect(cleanupMock).not.toHaveBeenCalled();
  });

  it("确认后调用 onCleanup 一次；成功则弹窗关闭", async () => {
    const mock = vi.fn().mockResolvedValue(true);
    await mount([issue("x")], [dead("E:/gone")], {
      onCleanup: mock as unknown as () => Promise<boolean>,
    });
    await act(async () => { findButton("一键清理")!.props.onClick(); });
    await act(async () => { confirmButton()!.props.onClick(); });
    expect(mock).toHaveBeenCalledTimes(1);
    expect(confirmButton()).toBeUndefined();
  });

  it("onCleanup 失败时弹窗保持打开，可重试", async () => {
    const mock = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    await mount([issue("x")], [dead("E:/gone")], {
      onCleanup: mock as unknown as () => Promise<boolean>,
    });
    await act(async () => { findButton("一键清理")!.props.onClick(); });
    await act(async () => { confirmButton()!.props.onClick(); });
    expect(mock).toHaveBeenCalledTimes(1);
    expect(confirmButton()).toBeDefined();
    // 重试第二次成功后关闭
    await act(async () => { confirmButton()!.props.onClick(); });
    expect(mock).toHaveBeenCalledTimes(2);
    expect(confirmButton()).toBeUndefined();
  });

  it("busy 时清理按钮处于 loading", async () => {
    await mount([issue("x")], [dead("E:/gone")], { busy: true });
    expect(findButton("一键清理")!.props.loading).toBe(true);
  });
});
