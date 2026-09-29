import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Button, Select, Checkbox, Modal, Title } from "@mantine/core";
import { open } from "@tauri-apps/plugin-dialog";
import ChatGptPanel from "./ChatGptPanel";
import { api, type ChatGptState, type ChatGptHistory, type ChatGptJob } from "../api";

vi.mock("@mantine/core", () => Object.fromEntries(["Alert", "Badge", "Button", "Card", "Checkbox", "Collapse", "Group", "Modal", "Select", "SimpleGrid", "Stack", "Text", "TextInput", "Title", "Tooltip"].map(name => [name, name.toLowerCase()])));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("./RiskConfirm", () => ({ default: "risk-confirm" }));
vi.mock("../api", () => ({ api: Object.fromEntries(["chatGptPreview", "chatGptPending", "chatGptBatchList", "chatGptBatchCreate", "chatGptBatchStep", "chatGptRecover", "chatGptOpenThread", "chatGptState", "chatGptHistory", "chatGptTransfer", "chatGptProfileAction", "chatGptCreateProfile", "chatGptSetInstallation", "chatGptDiagnose"].map(name => [name, vi.fn()])) }));
const state: ChatGptState = {
  installation: { path: "test-app", version: "test", compatible: true, detail: "test", cli: "test-cli" }, installationIssue: null,
  profiles: ["a", "b"].map(id => ({ id, name: id, createdAt: 1, directory: id, status: "stopped", pid: null, issue: null })),
  pickerExtensions: ["exe"], pickerTitle: "Choose", dataRoot: "test",
};
const history: ChatGptHistory = { warnings: [], complete: true, items: [{ key: "record", threadId: "thread", title: "Current record", workspace: "/project", revision: "v1", bytes: 10, modifiedAt: 1, transferable: true, detail: "Ready" }] };
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(r => { resolve = r; }); return { promise, resolve }; }
let renderer: ReactTestRenderer;
beforeEach(() => { vi.resetAllMocks(); vi.useFakeTimers(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.mocked(api.chatGptState).mockResolvedValue(state); vi.mocked(api.chatGptHistory).mockResolvedValue(history); vi.mocked(api.chatGptPending).mockResolvedValue([]); vi.mocked(api.chatGptBatchList).mockResolvedValue([]); vi.mocked(api.chatGptDiagnose).mockResolvedValue({ healthy: true, details: ["目录正常", "服务正常", "已登录"] }); vi.mocked(api.chatGptPreview).mockResolvedValue({ key: "record", title: "Current record", bytes: 10, images: 0, workspace: "/project", fingerprint: "hash" }); });
afterEach(() => { if (renderer) act(() => renderer.unmount()); vi.useRealTimers(); vi.unstubAllGlobals(); });
const button = (label: string) => renderer.root.findAllByType(Button).find(b => String(b.props.children).includes(label))!;
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

it("keeps client details in one toolbar, scrolls, and launches without a success banner", async () => {
  vi.mocked(api.chatGptProfileAction).mockResolvedValue({ ...state, profiles: state.profiles.map(p => p.id === "a" ? { ...p, status: "running" } : p) });
  await act(async () => { renderer = create(<ChatGptPanel />); });
  expect(renderer.root.findByProps({ className: "view-scroll chatgpt-scroll" })).toBeTruthy();
  const overview = renderer.root.findByProps({ className: "chatgpt-console" });
  expect(overview.findByProps({ className: "chatgpt-client-identity" })).toBeTruthy();
  expect(overview.findAllByProps({ className: "chatgpt-profile" })).toHaveLength(0);
  expect(overview.findAllByType(Button).some(item => item.props.children === "创建实例")).toBe(true);
  expect(renderer.root.findByProps({ className: "chatgpt-profile-section" }).findByProps({ className: "chatgpt-profile-list" }).findAllByProps({ className: "chatgpt-profile" })).toHaveLength(2);
  expect(renderer.root.findAllByProps({ className: "chatgpt-records-placeholder" })).toHaveLength(1);
  expect(renderer.root.findAllByType(Title).some(item => item.props.children === "ChatGPT 多开")).toBe(false);
  expect(renderer.root.findAll(node => typeof node.props.children === "string" && /\d+ 个实例|按次复制/.test(node.props.children))).toHaveLength(0);
  expect(renderer.root.findAllByType(Button).some(item => item.props.children === "关闭")).toBe(false);
  expect(renderer.root.findAllByProps({ className: "chatgpt-profile-path" })).toHaveLength(2);
  expect(renderer.root.findAllByProps({ className: "chatgpt-profile-directory" })).toHaveLength(0);
  expect(renderer.root.findAll(node => typeof node.props.className === "string" && node.props.className.includes("chatgpt-runtime-stopped"))).toHaveLength(2);
  expect(renderer.root.findAllByProps({ className: "chatgpt-runtime-line" })).toHaveLength(2);
  expect(renderer.root.findAllByProps({ className: "chatgpt-profile-actions" })[0].findAllByType(Button).every(item => item.props.className?.includes("chatgpt-profile-action"))).toBe(true);
  expect(renderer.root.findAllByType(Title).some(item => item.props.children === "跨账号会话同步")).toBe(true);
  expect(renderer.root.findAllByType(Button).some(item => String(item.props.children).includes("改名"))).toBe(false);
  await act(async () => { button("启动").props.onClick(); });
  expect(api.chatGptProfileAction).toHaveBeenCalledWith("a", "launch");
  expect(renderer.root.findAllByProps({ role: "status" }).some(item => String(item.props.children).includes("已打开实例"))).toBe(false);
});

it("offers close only for a running instance and sends the stop action", async () => {
  const running = { ...state, profiles: state.profiles.map(p => p.id === "a" ? { ...p, status: "running" as const } : p) };
  const result = deferred<ChatGptState>();
  vi.mocked(api.chatGptState).mockResolvedValue(running);
  vi.mocked(api.chatGptProfileAction).mockReturnValue(result.promise);
  await act(async () => { renderer = create(<ChatGptPanel />); });
  expect(renderer.root.findAllByType(Button).filter(item => item.props.children === "关闭")).toHaveLength(1);
  const status = renderer.root.findAllByProps({ className: "chatgpt-runtime-status chatgpt-runtime-running" });
  expect(status).toHaveLength(1);
  expect(status[0].props["aria-label"]).toBe("实例状态：运行中");
  expect(status[0].findAllByType("text")).toHaveLength(0);
  expect(renderer.root.findAllByProps({ className: "chatgpt-profile" })[0].findAllByType(Button).some(item => item.props.children === "删除")).toBe(false);
  act(() => { button("关闭").props.onClick(); });
  expect(button("关闭").props.children).toBe("关闭");
  expect(button("关闭").props["aria-busy"]).toBe(true);
  expect(button("关闭").props.disabled).toBeUndefined();
  await act(async () => { result.resolve(state); });
  expect(api.chatGptProfileAction).toHaveBeenCalledWith("a", "stop");
  expect(renderer.root.findAllByProps({ className: "chatgpt-feedback" })).toHaveLength(0);
});

it("checks profiles automatically without changing other buttons", async () => {
  const result = deferred<{ healthy: boolean; details: string[] }>();
  vi.mocked(api.chatGptDiagnose).mockReturnValueOnce(result.promise).mockResolvedValue({ healthy: true, details: ["目录正常", "服务正常", "已登录"] });
  await act(async () => { renderer = create(<ChatGptPanel />); });
  const profiles = renderer.root.findAllByProps({ className: "chatgpt-profile" });
  expect(renderer.root.findAllByType(Button).some(item => item.props.children === "检查隔离与账号")).toBe(false);
  expect(api.chatGptDiagnose).toHaveBeenCalledWith("a");
  expect(profiles[0].findByProps({ className: "chatgpt-health-label chatgpt-health-pending" })).toBeTruthy();
  expect(profiles[0].findAllByType(Button).find(item => item.props.children === "启动")!.props.disabled).toBe(false);
  expect(profiles[0].findAllByType(Button).find(item => item.props.children === "删除")!.props.disabled).toBeUndefined();
  expect(button("刷新").props.disabled).toBeUndefined();
  expect(button("创建实例").props.disabled).toBeUndefined();
  expect(select("来源实例").props.disabled).toBe(false);
  expect(renderer.root.findAllByProps({ className: "chatgpt-feedback" })).toHaveLength(0);
  await act(async () => { result.resolve({ healthy: true, details: ["独立目录检查通过", "官方服务使用此目录", "已登录账号"] }); });
  expect(profiles[0].findByProps({ className: "chatgpt-health-label chatgpt-health-ok" })).toBeTruthy();
  expect(profiles[1].findByProps({ className: "chatgpt-health-label chatgpt-health-ok" })).toBeTruthy();
  expect(profiles[0].findAll(node => typeof node.props.className === "string" && node.props.className.includes("chatgpt-runtime-healthy"))).toHaveLength(1);
  expect(profiles[0].findAllByProps({ className: "chatgpt-diagnostic" })).toHaveLength(0);
  expect(renderer.root.findAllByProps({ className: "chatgpt-feedback" })).toHaveLength(0);
});

it("opens details in a centered dialog only when an instance needs attention", async () => {
  vi.mocked(api.chatGptDiagnose).mockRejectedValueOnce(new Error("检查失败"));
  await act(async () => { renderer = create(<ChatGptPanel />); });
  const first = renderer.root.findAllByProps({ className: "chatgpt-profile" })[0];
  expect(first.findAllByProps({ className: "chatgpt-diagnostic" })).toHaveLength(0);
  expect(first.findAllByType(Button).find(item => item.props.children === "需处理")).toBeTruthy();
  expect(renderer.root.findAllByType(Modal).find(modal => String(modal.props.title).includes("检查详情"))!.props.opened).toBe(false);
  act(() => first.findAllByType(Button).find(item => item.props.children === "需处理")!.props.onClick());
  expect(renderer.root.findAllByType(Modal).find(modal => String(modal.props.title).includes("检查详情"))!.props.opened).toBe(true);
  expect(renderer.root.findAllByType("text").some(item => String(item.props.children).includes("检查失败"))).toBe(true);
  expect(renderer.root.findAllByProps({ className: "chatgpt-feedback" })).toHaveLength(0);
  await act(async () => { button("重新检查").props.onClick(); });
  expect(api.chatGptDiagnose).toHaveBeenCalledTimes(3);
  expect(first.findByProps({ className: "chatgpt-health-label chatgpt-health-ok" })).toBeTruthy();
  expect(renderer.root.findAllByType(Modal).find(modal => String(modal.props.title).includes("检查详情"))!.props.opened).toBe(false);
});

it("keeps two instance checks independent and hides healthy details", async () => {
  vi.mocked(api.chatGptDiagnose)
    .mockResolvedValueOnce({ healthy: false, details: ["目录正常", "服务正常", "尚未登录"] })
    .mockResolvedValueOnce({ healthy: true, details: ["目录正常", "服务正常", "已登录"] });
  await act(async () => { renderer = create(<ChatGptPanel />); });
  const profiles = renderer.root.findAllByProps({ className: "chatgpt-profile" });
  expect(api.chatGptDiagnose).toHaveBeenNthCalledWith(1, "a");
  expect(api.chatGptDiagnose).toHaveBeenNthCalledWith(2, "b");
  expect(profiles[0].findAllByType(Button).find(item => item.props.children === "需处理")).toBeTruthy();
  expect(profiles[1].findByProps({ className: "chatgpt-health-label chatgpt-health-ok" })).toBeTruthy();
  expect(renderer.root.findAllByType(Modal).find(modal => String(modal.props.title).includes("检查详情"))!.props.opened).toBe(false);
});

it("labels an isolated instance without a saved account as awaiting login", async () => {
  vi.mocked(api.chatGptDiagnose).mockResolvedValueOnce({ healthy: false, details: ["独立目录、配置与所有权检查通过。", "官方会话服务已确认使用此实例的数据目录。", "客户端未返回已登录账号，请在官方窗口完成登录。"] });
  await act(async () => { renderer = create(<ChatGptPanel />); });
  const first = renderer.root.findAllByProps({ className: "chatgpt-profile" })[0];
  expect(first.findAllByType(Button).find(item => item.props.children === "待登录")).toBeTruthy();
  expect(first.findAllByType(Button).some(item => item.props.children === "需处理")).toBe(false);
  expect(first.findAll(node => typeof node.props.className === "string" && node.props.className.includes("chatgpt-runtime-warning"))).toHaveLength(1);
});

it("queues two profile actions while only their own buttons show progress", async () => {
  const first = deferred<ChatGptState>();
  vi.mocked(api.chatGptProfileAction).mockReturnValueOnce(first.promise).mockResolvedValueOnce(state);
  await act(async () => { renderer = create(<ChatGptPanel />); });
  const profiles = renderer.root.findAllByProps({ className: "chatgpt-profile" });
  act(() => {
    profiles[0].findAllByType(Button).find(item => item.props.children === "启动")!.props.onClick();
    profiles[1].findAllByType(Button).find(item => item.props.children === "启动")!.props.onClick();
  });
  expect(profiles[0].findAllByType(Button).find(item => item.props.children === "启动")!.props["aria-busy"]).toBe(true);
  expect(profiles[1].findAllByType(Button).find(item => item.props.children === "启动")!.props["aria-busy"]).toBe(true);
  expect(button("刷新").props.disabled).toBeUndefined();
  expect(button("创建实例").props.disabled).toBeUndefined();
  expect(select("来源实例").props.disabled).toBe(false);
  await act(async () => { await Promise.resolve(); });
  expect(api.chatGptProfileAction).toHaveBeenCalledTimes(1);
  await act(async () => { first.resolve(state); });
  expect(api.chatGptProfileAction).toHaveBeenCalledTimes(2);
  expect(api.chatGptProfileAction).toHaveBeenNthCalledWith(2, "b", "launch");
});

it("keeps history buttons independent while their operations wait in order", async () => {
  const preview = deferred<Awaited<ReturnType<typeof api.chatGptPreview>>>();
  vi.mocked(api.chatGptPreview).mockReturnValue(preview.promise);
  await act(async () => { renderer = create(<ChatGptPanel />); });
  await act(async () => { select("来源实例").props.onChange("a"); select("目标实例").props.onChange("b"); });
  act(() => { button("复制到目标").props.onClick(); });
  expect(button("复制到目标").props["aria-busy"]).toBe(true);
  expect(button("选择目录").props.disabled).toBe(false);
  expect(button("重新读取记录").props.disabled).toBeUndefined();
  act(() => { button("选择目录").props.onClick(); });
  expect(button("选择目录").props["aria-busy"]).toBe(true);
  expect(open).not.toHaveBeenCalled();
  await act(async () => { preview.resolve({ key: "record", title: "Current record", bytes: 10, images: 0, workspace: "/project", fingerprint: "hash" }); });
  expect(open).toHaveBeenCalledTimes(1);
});

it("shows only the client and creation entry before the first instance exists", async () => {
  vi.mocked(api.chatGptState).mockResolvedValue({ ...state, profiles: [] });
  await act(async () => { renderer = create(<ChatGptPanel />); });
  expect(renderer.root.findByProps({ className: "chatgpt-page chatgpt-page-empty" })).toBeTruthy();
  expect(renderer.root.findByProps({ className: "chatgpt-onboarding" })).toBeTruthy();
  expect(button("创建第一个实例")).toBeTruthy();
  expect(renderer.root.findAllByProps({ className: "chatgpt-profile-section" })).toHaveLength(0);
  expect(renderer.root.findAllByProps({ className: "chatgpt-history" })).toHaveLength(0);
  expect(renderer.root.findAllByType(Select)).toHaveLength(1);
});

it("explains exactly what deleting an instance removes", async () => {
  await act(async () => { renderer = create(<ChatGptPanel />); });
  act(() => button("删除").props.onClick());
  const confirm = renderer.root.findAll(node => String(node.type) === "risk-confirm")[0];
  expect(confirm.props.opened).toBe(true);
  expect(confirm.props.consequences.join(" ")).toContain("登录数据");
  expect(confirm.props.consequences.join(" ")).toContain("外部项目文件");
  expect(confirm.props.consequences.join(" ")).toContain("云端账号");
  expect(api.chatGptProfileAction).not.toHaveBeenCalled();
});

it("late history from the previous source cannot replace the selected source", async () => {
  const first = deferred<ChatGptHistory>();
  vi.mocked(api.chatGptHistory).mockReturnValueOnce(first.promise).mockResolvedValueOnce(history);
  await act(async () => { renderer = create(<ChatGptPanel />); });
  act(() => select("来源实例").props.onChange("a"));
  await act(async () => { select("来源实例").props.onChange("b"); });
  await act(async () => { first.resolve({ warnings: ["STALE"], complete: true, items: [] }); });
  expect(renderer.root.findAllByType(Checkbox).map(node => node.props.label).join(" ")).not.toContain("STALE");
  expect(renderer.root.findAllByType(Checkbox).map(node => node.props.label).join(" ")).toContain("Current record");
  expect(renderer.root.findAllByProps({ className: "chatgpt-records" })).toHaveLength(1);
  expect(renderer.root.findAllByProps({ className: "chatgpt-records-placeholder" })).toHaveLength(0);
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
  await act(async () => { const click = button("创建独立副本").props.onClick; click(); click(); });
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

it("syncs only the chosen project and opens its newest copy", async () => {
  const other = { ...history.items[0], key: "other", threadId: "other", workspace: "/other", title: "Other project" };
  vi.mocked(api.chatGptHistory).mockResolvedValue({ ...history, items: [history.items[0], other] });
  const job = { id: "project-job", targetId: "b", requests: [{ sourceId: "a", targetId: "b", key: "record", revision: "v1", fingerprint: "hash" }], outcomes: [], cancelled: false };
  vi.mocked(api.chatGptBatchCreate).mockResolvedValue(job);
  vi.mocked(api.chatGptBatchStep).mockResolvedValue({ ...job, outcomes: [{ key: "record", result: { targetThreadId: "new-thread", duplicate: false, detail: "done" }, error: null }] });
  vi.mocked(api.chatGptOpenThread).mockResolvedValue(state);
  await act(async () => { renderer = create(<ChatGptPanel />); });
  await act(async () => { select("来源实例").props.onChange("a"); select("目标实例").props.onChange("b"); });
  act(() => select("同步范围").props.onChange("project"));
  act(() => select("选择项目").props.onChange("/project"));
  await act(async () => { button("同步并切换账号").props.onClick(); });
  expect(api.chatGptPreview).toHaveBeenCalledTimes(1);
  expect(api.chatGptBatchCreate).toHaveBeenCalledWith([job.requests[0]]);
  expect(api.chatGptOpenThread).toHaveBeenCalledWith("b", "new-thread");
});

it("does not close a running target until the user explicitly confirms", async () => {
  let running = true;
  vi.mocked(api.chatGptState).mockImplementation(async () => ({ ...state, profiles: state.profiles.map(profile =>
    profile.id === "b" ? { ...profile, status: running ? "running" : "stopped" } : profile) }));
  vi.mocked(api.chatGptProfileAction).mockImplementation(async (_id, action) => {
    if (action === "stop") running = false;
    return { ...state, profiles: state.profiles.map(profile =>
      profile.id === "b" ? { ...profile, status: running ? "running" : "stopped" } : profile) };
  });
  const job: ChatGptJob = { id: "confirmed", targetId: "b", requests: [{ sourceId: "a", targetId: "b", key: "record", revision: "v1", fingerprint: "hash" }], outcomes: [], cancelled: false };
  vi.mocked(api.chatGptBatchCreate).mockResolvedValue(job);
  vi.mocked(api.chatGptBatchStep).mockResolvedValue({ ...job, outcomes: [{ key: "record", result: { targetThreadId: "copied", duplicate: false, detail: "done" }, error: null }] });
  vi.mocked(api.chatGptOpenThread).mockResolvedValue(state);
  await act(async () => { renderer = create(<ChatGptPanel />); });
  await act(async () => { select("来源实例").props.onChange("a"); select("目标实例").props.onChange("b"); });
  act(() => renderer.root.findAllByType(Checkbox)[0].props.onChange({ currentTarget: { checked: true } }));
  await act(async () => { button("同步并切换账号").props.onClick(); });
  expect(api.chatGptProfileAction).not.toHaveBeenCalled();
  expect(renderer.root.findAllByType(Modal).find(modal => modal.props.title === "关闭目标窗口后同步？")!.props.opened).toBe(true);
  act(() => button("暂不关闭").props.onClick());
  expect(api.chatGptProfileAction).not.toHaveBeenCalled();
  expect(api.chatGptPreview).not.toHaveBeenCalled();
  expect(api.chatGptBatchCreate).not.toHaveBeenCalled();
  await act(async () => { button("同步并切换账号").props.onClick(); });
  await act(async () => {
    button("确认关闭并同步").props.onClick();
    await vi.advanceTimersByTimeAsync(500);
  });
  expect(api.chatGptProfileAction).toHaveBeenCalledWith("b", "stop");
  expect(api.chatGptBatchCreate).toHaveBeenCalledWith(job.requests);
  expect(api.chatGptOpenThread).toHaveBeenCalledWith("b", "copied");
});

it("syncs all local records through durable 100-item batches", async () => {
  const items = Array.from({ length: 101 }, (_, n) => ({ ...history.items[0], key: `record-${n}`, threadId: `thread-${n}`, title: `Record ${n}` }));
  vi.mocked(api.chatGptHistory).mockResolvedValue({ ...history, items });
  const jobs = new Map<string, ChatGptJob>();
  vi.mocked(api.chatGptBatchCreate).mockImplementation(async requests => {
    const job: ChatGptJob = { id: `job-${jobs.size}`, targetId: "b", requests, outcomes: [], cancelled: false };
    jobs.set(job.id, job);
    return job;
  });
  vi.mocked(api.chatGptBatchStep).mockImplementation(async (_targetId, id) => {
    const job = jobs.get(id)!;
    return { ...job, outcomes: job.requests.map(request => ({ key: request.key, result: { targetThreadId: `new-${request.key}`, duplicate: false, detail: "done" }, error: null })) };
  });
  vi.mocked(api.chatGptOpenThread).mockResolvedValue(state);
  await act(async () => { renderer = create(<ChatGptPanel />); });
  await act(async () => { select("来源实例").props.onChange("a"); select("目标实例").props.onChange("b"); });
  act(() => select("同步范围").props.onChange("all"));
  await act(async () => { button("同步并切换账号").props.onClick(); });
  expect(api.chatGptBatchCreate).toHaveBeenCalledTimes(2);
  expect(vi.mocked(api.chatGptBatchCreate).mock.calls.map(call => call[0].length)).toEqual([100, 1]);
  expect(api.chatGptOpenThread).toHaveBeenCalledWith("b", "new-record-0");
});

it("remembers the selected project for the same account pair", async () => {
  const saved = new Map<string, string>();
  vi.stubGlobal("localStorage", { getItem: (key: string) => saved.get(key) ?? null, setItem: (key: string, value: string) => { saved.set(key, value); } });
  await act(async () => { renderer = create(<ChatGptPanel />); });
  await act(async () => { select("来源实例").props.onChange("a"); select("目标实例").props.onChange("b"); });
  act(() => select("同步范围").props.onChange("project"));
  act(() => select("选择项目").props.onChange("/project"));
  expect(JSON.parse(saved.get("pathmux:chatgpt-sync:a:b")!).project).toBe("/project");
  act(() => renderer.unmount());
  await act(async () => { renderer = create(<ChatGptPanel />); });
  expect(select("来源实例").props.value).toBe("a");
  expect(select("目标实例").props.value).toBe("b");
  expect(select("同步范围").props.value).toBe("project");
  expect(select("选择项目").props.value).toBe("/project");
});
