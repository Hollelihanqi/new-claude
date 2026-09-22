import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import type { McpConnectionCheck } from "../../api";
import McpConnectionBadge from "./McpConnectionBadge";

vi.mock("@mantine/core", () => ({
  Badge: "span",
  Loader: "i",
  Text: "small",
  Tooltip: ({ children, label }: { children: ReactNode; label: ReactNode }) => (
    <div title={typeof label === "string" ? label : undefined}>{label}{children}</div>
  ),
}));

vi.mock("@tabler/icons-react", () => ({
  IconAlertCircle: "i",
  IconCircleCheck: "i",
  IconClock: "i",
}));

function render(checks: McpConnectionCheck[]) {
  let renderer!: ReactTestRenderer;
  act(() => {
    renderer = create(
      <McpConnectionBadge enabled supported checks={checks} busy={false} errors={[]} />
    );
  });
  return renderer;
}

describe("McpConnectionBadge", () => {
  it("prioritizes a failed environment over connected environments", () => {
    const renderer = render([
      { environment: "hq", name: "demo", status: "connected", detail: "✓ Connected" },
      { environment: "ds", name: "demo", status: "failed", detail: "✘ Failed to connect" },
    ]);
    const badges = renderer.root.findAllByType("span");
    expect(badges[badges.length - 1].children).toContain("连接失败");
    expect(renderer.root.findByType("small").children.join("")).toContain("ds：连接失败");
  });

  it("shows a healthy state only when every checked environment connected", () => {
    const renderer = render([
      { environment: "hq", name: "demo", status: "connected", detail: "✓ Connected" },
      { environment: "ds", name: "demo", status: "connected", detail: "✓ Connected" },
    ]);
    const badges = renderer.root.findAllByType("span");
    expect(badges[badges.length - 1].children).toContain("可连接");
  });

  it("does not report healthy when an environment probe itself failed", () => {
    let renderer!: ReactTestRenderer;
    act(() => {
      renderer = create(
        <McpConnectionBadge
          enabled
          supported
          checks={[{ environment: "hq", name: "demo", status: "connected", detail: "✓ Connected" }]}
          busy={false}
          errors={["ds：检查超时"]}
        />
      );
    });
    const badges = renderer.root.findAllByType("span");
    expect(badges[badges.length - 1].children).toContain("检测异常");
  });
});
