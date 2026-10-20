import type { PluginAction, PluginActionReport, PluginRow } from "../api";

export type PluginStep = { action: PluginAction } | { source: string };
export type PluginTaskStatus = "waiting" | "running" | "succeeded" | "failed" | "unverified";
export interface PluginTask {
  env: string;
  plugin: string;
  steps: PluginStep[];
  completed: number;
  status: PluginTaskStatus;
  detail: string;
  reloadHint: string;
  warnings: string[];
  expected: { installed: boolean; enabled?: boolean };
}

export interface PluginTaskExecutor {
  managePlugin(action: PluginAction, plugin: string, envs: string[], sharedScope: boolean): Promise<PluginActionReport>;
  installPluginPackage(source: string, envs: string[], sharedScope: boolean): Promise<PluginActionReport>;
  pluginsOverview(): Promise<PluginRow[]>;
}

export function pluginIdentity(name: string) {
  const separator = name.lastIndexOf("@");
  return {
    name: separator > 0 ? name.slice(0, separator) : name,
    source: separator > 0 ? name.slice(separator + 1) : "自定义来源",
  };
}

export function pluginStateLabel(row: PluginRow, env: string) {
  const state = row.envs.find((item) => item.env === env);
  return !state?.installed ? "未安装" : state.value ? "已启用" : "已停用";
}

export function createPluginTask(
  env: string, plugin: string, steps: PluginStep[], expected: PluginTask["expected"],
): PluginTask {
  return { env, plugin, steps, expected, completed: 0, status: "waiting", detail: "等待执行", reloadHint: "", warnings: [] };
}

export function planPluginCopy(row: PluginRow, sourceEnv: string, targets: string[]): PluginTask[] {
  const source = row.envs.find((entry) => entry.env === sourceEnv);
  if (!source?.installed) return [];
  const enabled = source.value === true;
  return [...new Set(targets)].filter((env) => env !== sourceEnv && env !== "__main__").map((env) => {
    const current = row.envs.find((entry) => entry.env === env);
    const steps: PluginStep[] = [];
    if (!current?.installed) steps.push({ action: "install" });
    if (!current?.installed || current.value !== enabled) steps.push({ action: enabled ? "enable" : "disable" });
    return createPluginTask(env, row.name, steps, { installed: true, enabled });
  });
}

export function validatePluginSource(mode: string, source: string): string | null {
  if (!source.trim()) return mode === "local" ? "请选择本地插件包" : "请输入插件来源，或选择已发现的插件";
  if (mode === "local") return null;
  if (/^https:\/\//i.test(source)) {
    try { if (new URL(source).hostname) return null; } catch { /* Fall through to the readable validation error. */ }
  }
  return /^[a-zA-Z0-9][a-zA-Z0-9._/-]*@[a-zA-Z0-9][a-zA-Z0-9._/-]*$/.test(source)
    ? null : "请输入 plugin@marketplace，或 HTTPS 插件 ZIP 地址";
}

const STEP_LABEL: Record<PluginAction, string> = {
  install: "正在安装插件", enable: "正在启用插件", disable: "正在停用插件", update: "正在更新插件", uninstall: "正在卸载插件",
};

/** Serialize writes while allowing unrelated plugins to be submitted. */
export class PluginCommandQueue {
  private tail: Promise<void> = Promise.resolve();
  private pending = new Set<string>();

  enqueue(keys: string[], run: () => Promise<void>, changed: (keys: string[]) => void): Promise<void> {
    if (keys.some((key) => this.pending.has(key))) return Promise.reject(new Error("部分目标插件正在处理，本次操作尚未提交；请等待完成后重试。"));
    keys.forEach((key) => this.pending.add(key));
    changed([...this.pending]);
    const next = this.tail.then(run).finally(() => {
      keys.forEach((key) => this.pending.delete(key));
      changed([...this.pending]);
    });
    this.tail = next.catch(() => {});
    return next;
  }
}

/** Each official command addresses one explicit environment. Completed steps survive retries. */
export async function executePluginTasks(
  previous: PluginTask[], executor: PluginTaskExecutor, onProgress: (tasks: PluginTask[]) => void,
): Promise<{ tasks: PluginTask[]; rows: PluginRow[] | null; readError: string }> {
  const tasks = previous.map((task) => ({ ...task, steps: [...task.steps], warnings: [...task.warnings] }));
  const active = tasks.filter((task) => task.status !== "succeeded");
  const publish = () => onProgress(tasks.map((task) => ({ ...task, warnings: [...task.warnings] })));
  for (const task of active) {
    if (!task.env || task.env === "__main__" || task.env === "__all__") {
      task.status = "failed"; task.detail = "请选择可编辑的受管理环境"; publish(); continue;
    }
    task.status = "running";
    while (task.completed < task.steps.length) {
      const step = task.steps[task.completed];
      task.detail = "source" in step ? "正在安装插件包" : STEP_LABEL[step.action];
      publish();
      try {
        if (task.completed > 0 && "action" in step && (step.action === "enable" || step.action === "disable")) {
          const current = (await executor.pluginsOverview()).find((row) => row.name === task.plugin)?.envs.find((entry) => entry.env === task.env);
          if (current?.installed && current.value === (step.action === "enable")) {
            task.completed += 1;
            task.detail = "已处于目标启停状态";
            continue;
          }
        }
        const report = "source" in step
          ? await executor.installPluginPackage(step.source, [task.env], false)
          : await executor.managePlugin(step.action, task.plugin, [task.env], false);
        const result = report.results.find((item) => item.env === task.env);
        if (!result?.ok) throw new Error(result?.detail ?? "未收到此环境的执行结果，请刷新状态后重试");
        task.plugin = report.plugin;
        const alreadyUpdated = task.steps.slice(0, task.completed).some((previous) => "action" in previous && previous.action === "update");
        if (!alreadyUpdated) task.reloadHint = report.reloadHint;
        if (report.policyWarning) task.warnings.push(report.policyWarning);
        task.completed += 1;
        task.detail = result.detail;
      } catch (error) {
        task.status = "failed";
        task.detail = String(error instanceof Error ? error.message : error);
        break;
      }
    }
    if (task.status !== "failed") { task.status = "unverified"; task.detail = "命令已完成，正在核对实际状态"; }
    publish();
  }
  try {
    const rows = await executor.pluginsOverview();
    for (const task of active.filter((item) => item.status === "unverified")) {
      const state = rows.find((row) => row.name === task.plugin)?.envs.find((entry) => entry.env === task.env);
      const matches = (state?.installed === true) === task.expected.installed
        && (!task.expected.installed || task.expected.enabled === undefined || state?.value === task.expected.enabled);
      task.status = matches ? "succeeded" : "unverified";
      task.detail = matches ? "已核对实际状态" : "命令已完成，但实际状态与预期不一致；请核对后再次检查";
    }
    publish();
    return { tasks, rows, readError: "" };
  } catch (error) {
    const readError = String(error);
    for (const task of active.filter((item) => item.status === "unverified")) task.detail = "命令已完成，读取实际状态失败；可重新核对，无需重复执行";
    publish();
    return { tasks, rows: null, readError };
  }
}
