import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Button, Select, Checkbox, Modal } from "@mantine/core";
import ChatGptPanel from "./ChatGptPanel";
import { api, type ChatGptState, type ChatGptHistory } from "../api";

vi.mock("@mantine/core", () => Object.fromEntries(["Alert", "Badge", "Button", "Card", "Checkbox", "Group", "Modal", "Select", "SimpleGrid", "Stack", "Text", "TextInput", "Title"].map(name => [name, name.toLowerCase()])));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("./RiskConfirm", () => ({ default: "risk-confirm" }));
vi.mock("../api", () => ({ api: Object.fromEntries(["chatGptPreview", "chatGptPending", "chatGptBatchList", "chatGptBatchCreate", "chatGptBatchStep", "chatGptRecover", "chatGptState", "chatGptHistory", "chatGptTransfer", "chatGptProfileAction", "chatGptCreateProfile", "chatGptSetInstallation"].map(name => [name, vi.fn()])) }));
const state: ChatGptState = {
  installation: { path: "test-app", version: "test", compatible: true, detail: "test", cli: "test-cli" }, installationIssue: null,
  profiles: ["a", "b"].map(id => ({ id, name: id, createdAt: 1, directory: id, status: "stopped", pid: null, issue: null })),
  pickerExtensions: ["exe"], pickerTitle: "Choose", dataRoot: "test",
};
const history: ChatGptHistory = { warnings: [], items: [{ key: "record", threadId: "thread", title: "Current record", revision: "v1", bytes: 10, modifiedAt: 1, transferable: true, detail: "Ready" }] };
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(r => { resolve = r; }); return { promise, resolve }; }
let renderer: ReactTestRenderer;
beforeEach(() => { vi.resetAllMocks(); vi.useFakeTimers(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.mocked(api.chatGptState).mockResolvedValue(state); vi.mocked(api.chatGptHistory).mockResolvedValue(history); vi.mocked(api.chatGptPending).mockResolvedValue([]); vi.mocked(api.chatGptBatchList).mockResolvedValue([]); vi.mocked(api.chatGptPreview).mockResolvedValue({ key: "record", title: "Current record", bytes: 10, images: 0, workspace: "/project", fingerprint: "hash" }); });
afterEach(() => { if (renderer) act(() => renderer.unmount()); vi.useRealTimers(); vi.unstubAllGlobals(); });
const button = (label: string) => renderer.root.findAllByType(Button).find(b => b.children.includes(label))!;
const select = (label: string) => renderer.root.findAllByType(Select).find(s => s.props.label === label)!;

it("inactive panels do not poll; returning refreshes state", async () => {
  await act(async () => { renderer = create(<ChatGptPanel active={false} />); });
  expect(api.chatGptState).not.toHaveBeenCalled();
  await act(async () => { renderer.update(<ChatGptPanel active />); });
  expect(api.chatGptState).toHaveBeenCalledTimes(1);
  await act(async () => { vi.advanceTimersByTime(15000); });
  expect(api.chatGptState).toHaveBeenCalledTimes(2);
  await act(async () => { renderer.update(<ChatGptPanel active={false} />); });
  await act(async () => { vi.advanceTimersByTime(30000); });
  expect(api.chatGptState).toHaveBeenCalledTimes(2);
});

it("late history from the previous source cannot replace the selected source", async () => {
  const first = deferred<ChatGptHistory>();
  vi.mocked(api.chatGptHistory).mockReturnValueOnce(first.promise).mockResolvedValueOnce(history);
  await act(async () => { renderer = create(<ChatGptPanel />); });
  act(() => select("来源实例").props.onChange("a"));
  await act(async () => { select("来源实例").props.onChange("b"); });
  await act(async () => { first.resolve({ warnings: ["STALE"], items: [] }); });
  expect(renderer.root.findAllByType(Checkbox).map(node => node.props.label).join(" ")).not.toContain("STALE");
  expect(renderer.root.findAllByType(Checkbox).map(node => node.props.label).join(" ")).toContain("Current record");
});

it("unsupported histories cannot be transferred", async () => {
  vi.mocked(api.chatGptHistory).mockResolvedValue({ ...history, items: [{ ...history.items[0], transferable: false }] });
  await act(async () => { renderer = create(<ChatGptPanel />); });
  await act(async () => { select("来源实例").props.onChange("a"); });
  act(() => select("目标实例").props.onChange("b"));
  expect(button("复制到目标").props.disabled).toBe(true);
  expect(api.chatGptTransfer).not.toHaveBeenCalled();
});

it("preview remains open and double clicks create only one durable queue", async () => {
  const waiting = deferred<Awaited<ReturnType<typeof api.chatGptBatchCreate>>>();
  vi.mocked(api.chatGptBatchCreate).mockReturnValue(waiting.promise);
  await act(async () => { renderer = create(<ChatGptPanel />); });
  await act(async () => { select("来源实例").props.onChange("a"); select("目标实例").props.onChange("b"); });
  await act(async () => { button("复制到目标").props.onClick(); });
  expect(api.chatGptPreview).toHaveBeenCalledWith({ sourceId: "a", targetId: "b", key: "record", revision: "v1" });
  expect(renderer.root.findAllByType(Modal).find(m => m.props.title === "确认复制范围")!.props.opened).toBe(true);
  expect(api.chatGptBatchCreate).not.toHaveBeenCalled();
  act(() => { const click = button("创建独立副本").props.onClick; click(); click(); });
  expect(api.chatGptBatchCreate).toHaveBeenCalledTimes(1);
  expect(api.chatGptBatchCreate).toHaveBeenCalledWith([{ sourceId: "a", targetId: "b", key: "record", revision: "v1", fingerprint: "hash" }]);
  await act(async () => { waiting.resolve({ id: "job", targetId: "b", requests: [], outcomes: [], cancelled: false }); });
});

it("restores a saved queue and submits the next step without a new import plan", async () => {
  const job = { id: "saved", targetId: "b", requests: [{ sourceId: "a", targetId: "b", key: "record", revision: "v1", fingerprint: "hash" }], outcomes: [], cancelled: false };
  vi.mocked(api.chatGptBatchList).mockResolvedValue([job]);
  vi.mocked(api.chatGptBatchStep).mockResolvedValue({ ...job, outcomes: [{ key: "record", result: { targetThreadId: "new", duplicate: true, detail: "done" }, error: null }] });
  await act(async () => { renderer = create(<ChatGptPanel />); });
  await act(async () => { select("目标实例").props.onChange("b"); });
  await act(async () => { button("继续队列").props.onClick(); });
  expect(api.chatGptBatchStep).toHaveBeenCalledWith("b", "saved");
  expect(api.chatGptBatchCreate).not.toHaveBeenCalled();
});

it("discard requires a separate confirmation and targets the pending copy", async () => {
  vi.mocked(api.chatGptPending).mockResolvedValue([{ key: "pending", title: "Interrupted", state: "pending" }]);
  vi.mocked(api.chatGptRecover).mockResolvedValue({ targetThreadId: "", duplicate: false, detail: "discarded" });
  await act(async () => { renderer = create(<ChatGptPanel />); });
  await act(async () => { select("目标实例").props.onChange("b"); });
  act(() => button("撤回未完成副本").props.onClick());
  expect(api.chatGptRecover).not.toHaveBeenCalled();
  await act(async () => { button("确认撤回").props.onClick(); });
  expect(api.chatGptRecover).toHaveBeenCalledWith("b", "pending", true);
});
