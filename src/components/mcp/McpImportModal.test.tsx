import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { describe, expect, it, vi } from "vitest";
import type { McpState } from "../../api";
import McpImportModal from "./McpImportModal";

vi.mock("@mantine/core", () => ({
  Alert: "div",
  Badge: "span",
  Box: "div",
  Button: "button",
  Checkbox: "input",
  Group: "div",
  Modal: "div",
  Select: "select",
  Stack: "div",
  Text: "span",
  Textarea: "textarea",
  Title: "h4",
}));

const emptyState: McpState = {
  services: [],
  instances: [],
  projects: [],
  revisions: {},
  issues: [],
  summary: {
    total: 0,
    enabled: 0,
    disabled: 0,
    warnings: 0,
    shadowed: 0,
  },
  operationWarnings: [],
  syncTargets: [],
  syncTargetRevisions: {},
};

function renderModal(opened: boolean) {
  return (
    <McpImportModal
      opened={opened}
      state={emptyState}
      onClose={vi.fn()}
      onSave={vi.fn().mockResolvedValue(undefined)}
    />
  );
}

describe("McpImportModal", () => {
  it("can transition from closed to open without changing the hook order", () => {
    let renderer: ReactTestRenderer;

    act(() => {
      renderer = create(renderModal(false));
    });

    expect(() => {
      act(() => {
        renderer.update(renderModal(true));
      });
    }).not.toThrow();
  });
});
