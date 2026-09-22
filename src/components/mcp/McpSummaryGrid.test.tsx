import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { describe, expect, it, vi } from "vitest";
import type { McpSummary } from "../../api";
import McpSummaryGrid from "./McpSummaryGrid";

vi.mock("@mantine/core", () => ({
  Card: "section",
  SimpleGrid: "div",
  Text: "span",
}));

const loadedSummary: McpSummary = {
  total: 4,
  enabled: 3,
  disabled: 1,
  warnings: 2,
  shadowed: 0,
};

describe("McpSummaryGrid", () => {
  it("keeps all four card slots mounted while the first load is pending", () => {
    let renderer!: ReactTestRenderer;

    act(() => {
      renderer = create(<McpSummaryGrid summary={undefined} sharedOverrideCount={undefined} />);
    });

    expect(renderer.root.findAllByType("section")).toHaveLength(4);
    expect(renderer.root.findAllByType("section").map((card) => (
      card.findAllByType("span").map((text) => text.children.join("")).join("")
    )))
      .toEqual(["全部定义—", "允许加载—", "存在警告—", "环境覆盖—"]);

    act(() => {
      renderer.update(<McpSummaryGrid summary={loadedSummary} sharedOverrideCount={2} />);
    });

    expect(renderer.root.findAllByType("section")).toHaveLength(4);
    expect(renderer.root.findAllByType("section")[3].findAllByType("span")
      .map((text) => text.children.join(""))).toEqual(["环境覆盖", "2"]);
    expect(renderer.root.findAllByType("section")[3].props.title)
      .toContain("保留了该环境自己的值");
  });
});
