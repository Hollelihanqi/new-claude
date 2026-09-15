import { describe, expect, it } from "vitest";
import type { Profile, ProfileRuntimeInfo } from "../api";
import { profileRuntimeStatus } from "./profileRuntimeStatus";

const profile = (overrides: Partial<Profile> = {}): Profile => ({
  name: "corp",
  type: "router",
  baseUrl: "https://gateway.example.test",
  hasToken: true,
  opusModel: "",
  sonnetModel: "",
  haikuModel: "",
  ...overrides,
});

const runtime = (overrides: Partial<ProfileRuntimeInfo> = {}): ProfileRuntimeInfo => ({
  name: "corp",
  configDir: "/tmp/corp",
  settingsExists: true,
  hasProjectData: false,
  authenticated: false,
  sharedDirsOk: true,
  ...overrides,
});

describe("environment runtime status", () => {
  it("reports extension migration separately from connection configuration", () => {
    expect(profileRuntimeStatus(profile(), runtime({ sharedDirsOk: false }), true)).toEqual({
      healthy: false,
      label: "扩展迁移未完成",
      shortLabel: "扩展待迁移",
    });
  });

  it("reports missing gateway credentials before extension state", () => {
    expect(profileRuntimeStatus(profile({ hasToken: false }), runtime({ sharedDirsOk: false }), true).label)
      .toBe("连接配置待完善");
  });

  it("distinguishes an account waiting for login", () => {
    expect(profileRuntimeStatus(profile({ type: "account" }), runtime(), true).label)
      .toBe("账户尚未登录");
  });

  it("is healthy only when CLI, access, and extension structure are ready", () => {
    expect(profileRuntimeStatus(profile(), runtime(), true)).toEqual({
      healthy: true,
      label: "环境正常",
      shortLabel: "正常",
    });
    expect(profileRuntimeStatus(profile(), runtime(), false).label).toBe("Claude CLI 未就绪");
  });

  it("reports gateway unreachable from the last full verification", () => {
    expect(profileRuntimeStatus(profile(), runtime(), true, true)).toEqual({
      healthy: false,
      gatewayDown: true,
      label: "网关未连通（上次诊断）",
      shortLabel: "网关异常",
    });
  });

  it("local config problems rank above the stale gateway conclusion", () => {
    // 本地配置没就绪时先报配置问题；网关结论只在其余全部正常时才浮出
    expect(profileRuntimeStatus(profile(), runtime({ sharedDirsOk: false }), true, true).label)
      .toBe("扩展迁移未完成");
    expect(profileRuntimeStatus(profile({ hasToken: false }), runtime(), true, true).label)
      .toBe("连接配置待完善");
  });

  it("shows a neutral checking state instead of green while probing", () => {
    expect(profileRuntimeStatus(profile(), runtime(), true, false, true)).toEqual({
      healthy: false,
      checking: true,
      label: "正在检测",
      shortLabel: "检测中",
    });
  });

  it("keeps confirmed problems visible while re-checking", () => {
    // 已有红色网关结论或本地配置问题时，重测中保持既有状态，不回退成「正在检测」
    expect(profileRuntimeStatus(profile(), runtime(), true, true, true).label)
      .toBe("网关未连通（上次诊断）");
    expect(profileRuntimeStatus(profile(), runtime({ sharedDirsOk: false }), true, false, true).label)
      .toBe("扩展迁移未完成");
  });
});
