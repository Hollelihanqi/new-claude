import { describe, expect, it, vi } from "vitest";
import type { PluginAction, PluginActionReport, PluginRow } from "../api";
import { createPluginTask, executePluginTasks, planPluginCopy, validatePluginSource, type PluginTaskExecutor } from "./pluginTasks";

function fixture() {
  const rows: PluginRow[] = [{ name: "demo@market", defaultInstalled: true, envs: [
    { env: "hq", installed: true, value: false, version: "1.0.0", inherited: false, reason: "" },
    { env: "ba", installed: false, value: null, inherited: false, reason: "" },
    { env: "ds", installed: false, value: null, inherited: false, reason: "" },
  ] }];
  const report = (env: string, action: PluginAction): PluginActionReport => ({ action, plugin: "demo@market", results: [{ env, ok: true, detail: "完成" }], reloadHint: action === "update" ? "需要重启会话" : "新会话生效" });
  const executor: PluginTaskExecutor = {
    managePlugin: vi.fn(async (action, _plugin, envs) => {
      const state = rows[0].envs.find((entry) => entry.env === envs[0])!;
      if (action === "install") { state.installed = true; state.value = true; state.version = "1.0.0"; }
      if (action === "enable") state.value = true;
      if (action === "disable") state.value = false;
      if (action === "uninstall") { state.installed = false; state.value = null; }
      if (action === "update") state.version = "1.0.1";
      return report(envs[0], action);
    }),
    installPluginPackage: vi.fn(async (_source, envs) => {
      const state = rows[0].envs.find((entry) => entry.env === envs[0])!;
      state.installed = true; state.value = true;
      return report(envs[0], "install");
    }),
    pluginsOverview: vi.fn(async () => structuredClone(rows)),
  };
  return { rows, executor, report };
}

describe("独立环境插件任务", () => {
  it("复制停用配置时只对缺失目标安装并停用，已有状态相同的环境无需执行", async () => {
    const { rows, executor } = fixture();
    const plan = planPluginCopy(rows[0], "hq", ["hq", "ba", "ba", "__main__"]);
    expect(plan.map((task) => task.env)).toEqual(["ba"]);
    const result = await executePluginTasks(plan, executor, () => {});
    expect(executor.managePlugin).toHaveBeenNthCalledWith(1, "install", "demo@market", ["ba"], false);
    expect(executor.managePlugin).toHaveBeenNthCalledWith(2, "disable", "demo@market", ["ba"], false);
    expect(rows[0].envs[0].value).toBe(false);
    expect(result.tasks[0].status).toBe("succeeded");
  });

  it("安装成功但停用失败后仅重试停用，不重复安装已成功的步骤与环境", async () => {
    const { rows, executor } = fixture();
    const original = executor.managePlugin;
    let fail = true;
    executor.managePlugin = vi.fn(async (action, plugin, envs, scope) => {
      if (envs[0] === "ds" && action === "disable" && fail) throw new Error("写入权限不足");
      return original(action, plugin, envs, scope);
    });
    const first = await executePluginTasks(planPluginCopy(rows[0], "hq", ["ba", "ds"]), executor, () => {});
    expect(first.tasks.map((task) => task.status)).toEqual(["succeeded", "failed"]);
    expect(first.tasks[1].completed).toBe(1);
    const callsBefore = vi.mocked(executor.managePlugin).mock.calls.length;
    fail = false;
    const retry = await executePluginTasks(first.tasks, executor, () => {});
    expect(vi.mocked(executor.managePlugin).mock.calls.slice(callsBefore)).toEqual([["disable", "demo@market", ["ds"], false]]);
    expect(retry.tasks.every((task) => task.status === "succeeded")).toBe(true);
  });

  it("读取结果失败时保留执行记录，再次核对不重复执行命令", async () => {
    const { executor } = fixture();
    vi.mocked(executor.pluginsOverview).mockRejectedValueOnce(new Error("读取失败"));
    const first = await executePluginTasks([createPluginTask("hq", "demo@market", [{ action: "enable" }], { installed: true, enabled: true })], executor, () => {});
    expect(first.tasks[0].status).toBe("unverified");
    const retry = await executePluginTasks(first.tasks, executor, () => {});
    expect(executor.managePlugin).toHaveBeenCalledTimes(1);
    expect(retry.tasks[0].status).toBe("succeeded");
  });

  it("官方命令成功但实际状态不匹配时不宣称完成", async () => {
    const { executor, report } = fixture();
    vi.mocked(executor.managePlugin).mockResolvedValue(report("hq", "enable"));
    const result = await executePluginTasks([createPluginTask("hq", "demo@market", [{ action: "enable" }], { installed: true, enabled: true })], executor, () => {});
    expect(result.tasks[0].status).toBe("unverified");
  });

  it("更新后保留原停用状态，同时保留重启会话的真实提示", async () => {
    const { executor, rows } = fixture();
    const result = await executePluginTasks([createPluginTask("hq", "demo@market", [{ action: "update" }, { action: "disable" }], { installed: true, enabled: false })], executor, () => {});
    expect(rows[0].envs[0]).toMatchObject({ value: false, version: "1.0.1" });
    expect(result.tasks[0].reloadHint).toContain("需要重启会话");
    expect(result.tasks[0].reloadHint).not.toContain("新会话生效");
    expect(result.tasks[0].status).toBe("succeeded");
  });

  it.each(["C:\\Users\\hq\\Downloads\\plugin.zip", "/Users/hq/Downloads/plugin.zip"])("Windows/macOS 本地包路径原样传递：%s", async (source) => {
    const { executor } = fixture();
    const result = await executePluginTasks([createPluginTask("ba", "待识别的插件包", [{ source }, { action: "disable" }], { installed: true, enabled: false })], executor, () => {});
    expect(executor.installPluginPackage).toHaveBeenCalledWith(source, ["ba"], false);
    expect(executor.managePlugin).toHaveBeenCalledWith("disable", "demo@market", ["ba"], false);
    expect(result.tasks[0].plugin).toBe("demo@market");
    expect(result.tasks[0].status).toBe("succeeded");
  });

  it.each(["__main__", "__all__", ""])("不能向只读或聚合目标执行命令：%s", async (env) => {
    const { executor } = fixture();
    const result = await executePluginTasks([createPluginTask(env, "demo@market", [{ action: "uninstall" }], { installed: false })], executor, () => {});
    expect(executor.managePlugin).not.toHaveBeenCalled();
    expect(result.tasks[0].status).toBe("failed");
  });

  it("拒绝缺少来源的安装输入", () => {
    expect(validatePluginSource("remote", "")).toBeTruthy();
    expect(validatePluginSource("remote", "http://example.com/a.zip")).toBeTruthy();
    expect(validatePluginSource("remote", "https://example.com/a.zip")).toBeNull();
    expect(validatePluginSource("remote", "demo@market")).toBeNull();
    expect(validatePluginSource("local", "/Users/hq/plugin.zip")).toBeNull();
  });
});
