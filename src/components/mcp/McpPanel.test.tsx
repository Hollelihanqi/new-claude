import { StrictMode } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api, type McpState } from "../../api";
import PersistentPage from "../PersistentPage";
import StableRefreshButton from "../StableRefreshButton";
import McpPanel from "./McpPanel";

vi.mock("@mantine/core", () => {
  const passthrough = ({ children }: { children?: React.ReactNode }) => children;
  const compound = (names: string[]) => Object.assign(passthrough.bind(null), Object.fromEntries(names.map(name => [name, passthrough])));
  return {
    ...Object.fromEntries(["Alert", "Badge", "Box", "Button", "Card", "Code", "Group", "Loader", "Modal", "Select", "SimpleGrid", "Stack", "Switch", "Text", "TextInput", "Tooltip"].map(name => [name, name.toLowerCase()])),
    Table: compound(["Thead", "Tbody", "Tr", "Th", "Td"]),
    Tabs: compound(["List", "Tab", "Panel"]),
    Menu: compound(["Target", "Dropdown", "Item", "Divider"]),
  };
});
vi.mock("@mantine/notifications", () => ({ notifications: { show: vi.fn() } }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("../FeatureHelp", () => ({ default: () => null }));
vi.mock("../PageErrorBoundary", () => ({ default: ({ children }: { children?: React.ReactNode }) => children }));
vi.mock("../StableRefreshButton", () => ({ default: () => null }));
vi.mock("./McpServiceDrawer", () => ({ default: () => null }));
vi.mock("./McpImportModal", () => ({ default: () => null }));
vi.mock("./McpSourceIssuesCard", () => ({ default: () => null, cleanupOutcomeView: vi.fn() }));
vi.mock("../../api", () => ({ api: {
  listMcpServices: vi.fn(), listMcpUpdateInfo: vi.fn(), probeMcpConnections: vi.fn(), checkMcpUpdates: vi.fn(),
} }));

const state: McpState = {
  services: [], instances: [], projects: [], revisions: {}, issues: [], deadEntries: [],
  summary: { total: 0, enabled: 0, disabled: 0, warnings: 0, shadowed: 0 },
  operationWarnings: [], syncTargets: [], syncTargetRevisions: {}, sharedOverrides: [],
};
let renderer: ReactTestRenderer | undefined;
let now = 1_000_000;
const page = (active: boolean) => <StrictMode><PersistentPage active={active} warmupDelay={0}><McpPanel /></PersistentPage></StrictMode>;
const mount = async (active: boolean) => {
  await act(async () => { renderer = create(page(active)); });
  // Background preloading mounts the page even while it is hidden.
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 10)); });
};
const show = async (active: boolean) => { await act(async () => { renderer!.update(page(active)); }); };

beforeEach(() => {
  vi.resetAllMocks();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  now += 100_000;
  vi.spyOn(Date, "now").mockImplementation(() => now);
  vi.mocked(api.listMcpServices).mockResolvedValue(state);
  vi.mocked(api.listMcpUpdateInfo).mockResolvedValue({ entries: [], errors: [] });
  vi.mocked(api.probeMcpConnections).mockResolvedValue({ checks: [], errors: [] });
  vi.mocked(api.checkMcpUpdates).mockResolvedValue({ entries: [], errors: [] });
});
afterEach(async () => {
  if (renderer) await act(async () => { renderer!.unmount(); });
  renderer = undefined;
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("does not probe during hidden warmup, but probes on first entry without a click", async () => {
  await mount(false);
  expect(api.listMcpServices).toHaveBeenCalled();
  expect(api.probeMcpConnections).not.toHaveBeenCalled();
  await show(true);
  expect(api.probeMcpConnections).toHaveBeenCalledTimes(1);
  expect(api.checkMcpUpdates).not.toHaveBeenCalled();
});

it("probes on an initially visible mount once, including StrictMode effect replay", async () => {
  await mount(true);
  expect(api.probeMcpConnections).toHaveBeenCalledTimes(1);
});

it("rechecks automatically on reentry after 30 seconds and avoids rapid switching duplicates", async () => {
  await mount(true);
  await show(false);
  now += 29_999;
  await show(true);
  expect(api.probeMcpConnections).toHaveBeenCalledTimes(1);
  await show(false);
  now += 1;
  await show(true);
  expect(api.probeMcpConnections).toHaveBeenCalledTimes(2);
});

it("does not queue duplicate automatic probes while one is still pending", async () => {
  let finish!: (value: { checks: []; errors: [] }) => void;
  vi.mocked(api.probeMcpConnections).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await mount(true);
  await show(false);
  now += 31_000;
  await show(true);
  expect(api.probeMcpConnections).toHaveBeenCalledTimes(1);
  await act(async () => { finish({ checks: [], errors: [] }); });
  await show(false);
  await show(true);
  expect(api.probeMcpConnections).toHaveBeenCalledTimes(1);
});

it("retries a failed check on a later entry without requiring a manual click", async () => {
  vi.mocked(api.probeMcpConnections).mockRejectedValueOnce(new Error("temporary failure"));
  await mount(true);
  await show(false);
  now += 30_000;
  await show(true);
  expect(api.probeMcpConnections).toHaveBeenCalledTimes(2);
});

it("retains manual detection even when the automatic result is still fresh", async () => {
  await mount(true);
  await act(async () => { await renderer!.root.findByType(StableRefreshButton).props.onClick(); });
  expect(api.probeMcpConnections).toHaveBeenCalledTimes(2);
  expect(api.checkMcpUpdates).toHaveBeenCalledTimes(1);
});
