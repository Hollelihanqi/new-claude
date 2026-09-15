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
});
