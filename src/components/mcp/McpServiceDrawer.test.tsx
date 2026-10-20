import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Button } from "@mantine/core";
import { api, type McpService, type McpState } from "../../api";
import McpServiceDrawer from "./McpServiceDrawer";

vi.mock("@mantine/core", () => {
  const components = Object.fromEntries(["Alert", "Badge", "Box", "Button", "Drawer", "Fieldset", "Group", "PasswordInput", "Select", "Stack", "Switch", "Text", "TextInput", "Textarea", "Title"].map(name => [name, name.toLowerCase()]));
  return { ...components, Tabs: Object.assign(({ children }: { children?: React.ReactNode }) => children, { List: "tab-list", Tab: "tab", Panel: "tab-panel" }) };
});
vi.mock("../FeatureHelp", () => ({ default: () => null }));
vi.mock("../../api", () => ({ api: { readMcpServiceConfig: vi.fn() } }));

const service: McpService = {
  locator: { scope: "user", name: "demo" }, transport: "stdio", config: { command: "node", env: { TOKEN: "[REDACTED]" } },
  enabled: true, effectiveState: "effective", shadowedBy: [], shadowedContextCount: 0,
  sourceId: "user:__main__", revision: "revision", sensitivePaths: ["/env/TOKEN"], warnings: [],
};
const state: McpState = {
  services: [service], instances: [], projects: [], revisions: {}, issues: [], deadEntries: [],
  summary: { total: 1, enabled: 1, disabled: 0, warnings: 0, shadowed: 0 }, operationWarnings: [], syncTargets: [], syncTargetRevisions: {}, sharedOverrides: [],
};
const raw = { command: "node", env: { TOKEN: "secret-for-edit-only" }, custom: { retained: true } };
let renderer: ReactTestRenderer;
beforeEach(() => {
  vi.resetAllMocks();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(api.readMcpServiceConfig).mockResolvedValue(raw);
});
afterEach(() => { if (renderer) act(() => renderer.unmount()); vi.unstubAllGlobals(); });

it.each(["edit", "copy"] as const)("fetches raw values only for an open %s drawer and preserves them on save", async (mode) => {
  const save = vi.fn(async () => {});
  const props = { mode, service, state, onClose: vi.fn(), onSave: save, onTest: vi.fn() };
  await act(async () => { renderer = create(<McpServiceDrawer {...props} opened={false} />); });
  expect(api.readMcpServiceConfig).not.toHaveBeenCalled();
  await act(async () => { renderer.update(<McpServiceDrawer {...props} opened />); });
  expect(api.readMcpServiceConfig).toHaveBeenCalledWith(service.locator, "revision");
  const renderedStrings = renderer.root.findAll(() => true).flatMap(node => Object.values(node.props).filter(value => typeof value === "string"));
  expect(renderedStrings.join("\n")).not.toContain("secret-for-edit-only");
  await act(async () => { renderer.root.findAllByType(Button).find(button => button.props.children === "保存")!.props.onClick(); });
  expect(save).toHaveBeenCalledWith(expect.objectContaining({ config: raw }));
  await act(async () => { renderer.update(<McpServiceDrawer {...props} opened={false} />); });
  await act(async () => { renderer.update(<McpServiceDrawer {...props} opened />); });
  expect(api.readMcpServiceConfig).toHaveBeenCalledTimes(2);
});

it("rejects a failed or stale config read without offering to save placeholders", async () => {
  vi.mocked(api.readMcpServiceConfig).mockRejectedValue(new Error("服务配置已变化"));
  await act(async () => { renderer = create(<McpServiceDrawer opened mode="edit" service={service} state={state} onClose={vi.fn()} onSave={vi.fn()} onTest={vi.fn()} />); });
  expect(JSON.stringify(renderer.toJSON())).toContain("服务配置已变化");
  expect(renderer.root.findAllByType(Button).some(button => button.props.children === "保存")).toBe(false);
});
