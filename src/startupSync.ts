// 启动同步的锁竞争重试。升级后首次启动最容易撞上：旧进程/安装器收尾的同步
// 刚死、孤儿锁还没过 180 秒回收期（sync.rs LOCK_STALE_SECS）。这种失败是纯时序
// 问题，重试必然成功；只有重试耗尽或其他错误才需要弹窗打扰用户。

export const STARTUP_SYNC_RETRY_DELAYS_MS = [15_000, 45_000, 120_000];

export function isLockBusyError(error: unknown): boolean {
  return String(error).includes("配置正在同步");
}

export interface StartupSyncOutcome {
  ok: boolean;
  retried: number;
  error?: unknown;
}

export async function syncAllWithRetry(
  invokeSync: () => Promise<string>,
  sleep: (ms: number) => Promise<void> = (ms) =>
    new Promise((resolve) => setTimeout(resolve, ms)),
): Promise<StartupSyncOutcome> {
  let attempt = 0;
  for (;;) {
    try {
      await invokeSync();
      return { ok: true, retried: attempt };
    } catch (error) {
      if (!isLockBusyError(error) || attempt >= STARTUP_SYNC_RETRY_DELAYS_MS.length) {
        return { ok: false, retried: attempt, error };
      }
      await sleep(STARTUP_SYNC_RETRY_DELAYS_MS[attempt]);
      attempt += 1;
    }
  }
}
