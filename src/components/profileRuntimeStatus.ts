import type { Profile, ProfileRuntimeInfo } from "../api";

export interface ProfileStatus {
  healthy: boolean;
  label: string;
  shortLabel: string;
}

export function profileRuntimeStatus(
  profile: Profile,
  runtime: ProfileRuntimeInfo | undefined,
  claudeFound: boolean
): ProfileStatus {
  if (!claudeFound) {
    return { healthy: false, label: "Claude CLI 未就绪", shortLabel: "CLI 未就绪" };
  }

  const accessReady = profile.type === "router"
    ? !!profile.baseUrl && profile.hasToken
    : !!runtime?.authenticated;
  if (!accessReady) {
    return profile.type === "router"
      ? { healthy: false, label: "连接配置待完善", shortLabel: "连接待完善" }
      : { healthy: false, label: "账户尚未登录", shortLabel: "等待登录" };
  }

  if (runtime?.sharedDirsOk === false) {
    return { healthy: false, label: "扩展迁移未完成", shortLabel: "扩展待迁移" };
  }

  return { healthy: true, label: "环境正常", shortLabel: "正常" };
}
