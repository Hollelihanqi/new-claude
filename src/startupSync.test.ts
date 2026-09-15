import { describe, expect, it, vi } from "vitest";
import {
  STARTUP_SYNC_RETRY_DELAYS_MS,
  isLockBusyError,
  syncAllWithRetry,
} from "./startupSync";

const busy = () => Promise.reject(new Error("配置正在同步，请稍后重试"));

describe("启动同步锁竞争重试", () => {
  it("锁竞争失败会按退避间隔重试，成功后不再重试", async () => {
    const sleeps: number[] = [];
    let calls = 0;
    const outcome = await syncAllWithRetry(
      () => (calls++ < 2 ? busy() : Promise.resolve("ok")),
      async (ms) => {
        sleeps.push(ms);
      },
    );
    expect(outcome).toEqual({ ok: true, retried: 2 });
    expect(calls).toBe(3);
    expect(sleeps).toEqual(STARTUP_SYNC_RETRY_DELAYS_MS.slice(0, 2));
  });

  it("非锁竞争错误立即失败，不重试", async () => {
    const sleep = vi.fn(async () => {});
    const outcome = await syncAllWithRetry(
      () => Promise.reject(new Error("磁盘空间不足")),
      sleep,
    );
    expect(outcome.ok).toBe(false);
    expect(outcome.retried).toBe(0);
    expect(String(outcome.error)).toContain("磁盘空间不足");
    expect(sleep).not.toHaveBeenCalled();
  });

  it("锁竞争重试耗尽后如实失败并保留最后错误", async () => {
    const sleep = vi.fn(async () => {});
    const outcome = await syncAllWithRetry(busy, sleep);
    expect(outcome.ok).toBe(false);
    expect(outcome.retried).toBe(STARTUP_SYNC_RETRY_DELAYS_MS.length);
    expect(sleep).toHaveBeenCalledTimes(STARTUP_SYNC_RETRY_DELAYS_MS.length);
    expect(String(outcome.error)).toContain("配置正在同步");
  });

  it("isLockBusyError 只认配置锁冲突这一种可重试错误", () => {
    expect(isLockBusyError(new Error("配置正在同步，请稍后重试"))).toBe(true);
    expect(isLockBusyError(new Error("网关超时"))).toBe(false);
    expect(isLockBusyError(undefined)).toBe(false);
  });
});
