import { act, create } from "react-test-renderer";
import { afterEach, describe, expect, it, vi } from "vitest";
import StableRefreshButton from "./StableRefreshButton";

vi.mock("@mantine/core", () => ({ Button: "button", Loader: "i" }));
vi.mock("@tabler/icons-react", () => ({ IconRefresh: "svg" }));

describe("StableRefreshButton", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("keeps its label visible and width stable while busy", () => {
    let renderer!: ReturnType<typeof create>;
    const onClick = vi.fn();

    act(() => {
      renderer = create(
        <StableRefreshButton busy label="刷新状态" onClick={onClick} />,
      );
    });

    const button = renderer.root.findByType("button");
    expect(button.props.loading).toBeUndefined();
    expect(button.props.disabled).toBeUndefined();
    expect(button.props["aria-disabled"]).toBe(true);
    expect(button.findByProps({ className: "stable-refresh-label idle" }).props["aria-hidden"])
      .toBe(true);
    expect(button.findByProps({ className: "stable-refresh-label busy" }).children)
      .toContain("刷新中…");
    expect(button.props.leftSection.type).toBe("i");
    expect(button.props.leftSection.props.className).toBe("stable-refresh-loader");

    act(() => button.props.onClick());
    expect(onClick).not.toHaveBeenCalled();
  });

  it("keeps fast refresh feedback visible for a perceptible minimum duration", () => {
    vi.useFakeTimers();
    let renderer!: ReturnType<typeof create>;
    const onClick = vi.fn();

    act(() => {
      renderer = create(
        <StableRefreshButton busy={false} label="Refresh" onClick={onClick} />,
      );
    });

    act(() => renderer.root.findByType("button").props.onClick());
    expect(onClick).toHaveBeenCalledOnce();

    act(() => {
      renderer.update(
        <StableRefreshButton busy label="Refresh" onClick={onClick} />,
      );
      renderer.update(
        <StableRefreshButton busy={false} label="Refresh" onClick={onClick} />,
      );
      vi.advanceTimersByTime(649);
    });

    expect(renderer.root.findByType("button").props["aria-busy"]).toBe(true);

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(renderer.root.findByType("button").props["aria-busy"]).toBe(false);
  });
});
