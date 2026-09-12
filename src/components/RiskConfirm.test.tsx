import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import RiskConfirm from "./RiskConfirm";

// Mantine 组件替换成同名小写宿主元素，便于直接读 props 断言。
// Modal 在这里不做开合裁剪 —— 断言只关心"打开时"的行为，调用方传 opened。
vi.mock(
  "@mantine/core",
  () =>
    Object.fromEntries(
      ["Alert", "Button", "Checkbox", "Group", "Modal", "Stack", "Text"].map((name) => [
        name,
        name.toLowerCase(),
      ])
    )
);
vi.mock("@tabler/icons-react", () => ({
  IconAlertTriangle: () => null,
  IconShieldLock: () => null,
}));

let renderer: ReactTestRenderer;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
});
afterEach(() => {
  if (renderer) act(() => renderer.unmount());
  vi.unstubAllGlobals();
});

const nodesOfType = (type: string) =>
  renderer.root.findAll((node) => node.type === (type as never));
const buttonLabelled = (label: string) =>
  nodesOfType("button").find((node) => node.props.children === label)!;

const mount = async (props: Partial<Parameters<typeof RiskConfirm>[0]> = {}) => {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  await act(async () => {
    renderer = create(
      <RiskConfirm
        opened
        level="critical"
        title="开启「跳过权限确认」"
        consequences={["该环境内的文件修改与命令执行不再二次确认。"]}
        confirmLabel="确认开启"
        onConfirm={onConfirm}
        onCancel={onCancel}
        {...props}
      />
    );
  });
  return { onConfirm, onCancel };
};

it("critical 级：未勾选不能确认，勾选后才放行", async () => {
  const { onConfirm } = await mount();
  expect(buttonLabelled("确认开启").props.disabled).toBe(true);

  await act(async () => {
    nodesOfType("checkbox")[0].props.onChange({ currentTarget: { checked: true } });
  });
  expect(buttonLabelled("确认开启").props.disabled).toBe(false);

  await act(async () => {
    buttonLabelled("确认开启").props.onClick();
  });
  expect(onConfirm).toHaveBeenCalledTimes(1);
});

it("critical 级：关闭时重置勾选，避免下次误触直接放行", async () => {
  const { onCancel } = await mount();
  await act(async () => {
    nodesOfType("checkbox")[0].props.onChange({ currentTarget: { checked: true } });
  });
  expect(buttonLabelled("确认开启").props.disabled).toBe(false);

  await act(async () => {
    buttonLabelled("取消").props.onClick();
  });
  expect(onCancel).toHaveBeenCalledTimes(1);
  // 重新打开时必须是"又回到未勾选"，否则上一次的勾选会把这次一开就放行
  expect(buttonLabelled("确认开启").props.disabled).toBe(true);
});

it("外部把 opened 置 false 再打开时，勾选必须已重置", async () => {
  // 回归（审查 F8）：父组件可能**不走 onCancel**、直接把 opened 置 false。
  // 此时 close() 不会被调用 —— 只在 close() 里重置的话，勾选会残留到下次打开，
  // 用户再打开时一次点击就能确认，等于 `critical` 的二次确认形同虚设。
  const props = {
    level: "critical" as const,
    title: "开启「跳过权限确认」",
    consequences: ["该环境内的文件修改与命令执行不再二次确认。"],
    confirmLabel: "确认开启",
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
  };
  await act(async () => {
    renderer = create(<RiskConfirm opened {...props} />);
  });
  await act(async () => {
    nodesOfType("checkbox")[0].props.onChange({ currentTarget: { checked: true } });
  });
  expect(buttonLabelled("确认开启").props.disabled).toBe(false);

  // 外部关闭（不触发 onCancel），再打开
  await act(async () => {
    renderer.update(<RiskConfirm opened={false} {...props} />);
  });
  await act(async () => {
    renderer.update(<RiskConfirm opened {...props} />);
  });
  expect(
    buttonLabelled("确认开启").props.disabled,
    "外部关闭后重开仍处于已勾选状态 —— 二次确认被绕过"
  ).toBe(true);
  expect(props.onCancel).not.toHaveBeenCalled();
});

it("high 级：不要求勾选，确认按钮直接可用", async () => {
  const { onConfirm } = await mount({ level: "high", confirmLabel: "确认清空" });
  expect(nodesOfType("checkbox")).toHaveLength(0);
  expect(buttonLabelled("确认清空").props.disabled).toBe(false);

  await act(async () => {
    buttonLabelled("确认清空").props.onClick();
  });
  expect(onConfirm).toHaveBeenCalledTimes(1);
});

it("逐条渲染具体后果 —— 泛泛的「确定吗？」不构成确认", async () => {
  const consequences = ["将移除全部 3 张已导入证书，无法撤销。", "自签网关会立刻连不上。"];
  await mount({ consequences, confirmLabel: "确认清空" });
  const texts = nodesOfType("text").flatMap((node) => node.children);
  for (const line of consequences) {
    expect(texts.some((text) => typeof text === "string" && text.includes(line))).toBe(true);
  }
});
