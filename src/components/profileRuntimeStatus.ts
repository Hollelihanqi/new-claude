import type { Profile, ProfileRuntimeInfo } from "../api";

export interface ProfileStatus {
  healthy: boolean;
  label: string;
  shortLabel: string;
  /** 网关异常态（来自最近一次完整诊断）：环境页要为它提供单环境「检测」按钮 */
  gatewayDown?: boolean;
}

export function profileRuntimeStatus(
  profile: Profile,
  runtime: ProfileRuntimeInfo | undefined,
  claudeFound: boolean,
  gatewayDown = false
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

  // 本地配置全部就绪，但最近一次诊断发现该网关不可达。列表页不做实时探测，
  // 只消费诊断结论 —— 是否已恢复由用户点「检测」单环境复测确认。
  if (gatewayDown) {
    return {
      healthy: false,
      gatewayDown: true,
      label: "网关未连通（上次诊断）",
      shortLabel: "网关异常",
    };
  }

  return { healthy: true, label: "环境正常", shortLabel: "正常" };
}
