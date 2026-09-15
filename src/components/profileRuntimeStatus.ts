import type { Profile, ProfileRuntimeInfo } from "../api";

export interface ProfileStatus {
  healthy: boolean;
  label: string;
  shortLabel: string;
  /** 网关异常态（来自最近一次完整诊断）：环境页要为它提供单环境「检测」按钮 */
  gatewayDown?: boolean;
  /** 检测进行中且尚无探测结论：显示中性「正在检测」，绿色必须有结论背书 */
  checking?: boolean;
}

export function profileRuntimeStatus(
  profile: Profile,
  runtime: ProfileRuntimeInfo | undefined,
  claudeFound: boolean,
  gatewayDown = false,
  checking = false
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
  // 检测中保留旧结论（红），复测通过/失败的切换由检测按钮驱动。
  if (gatewayDown) {
    return {
      healthy: false,
      gatewayDown: true,
      label: "网关未连通（上次诊断）",
      shortLabel: "网关异常",
    };
  }

  // 其余全部就绪、但探测还没出结论（升级/安装后首次自动检测进行中）：
  // 绿色必须由探测结论背书，检测中显示中性态。
  if (checking) {
    return { healthy: false, checking: true, label: "正在检测", shortLabel: "检测中" };
  }

  return { healthy: true, label: "环境正常", shortLabel: "正常" };
}
