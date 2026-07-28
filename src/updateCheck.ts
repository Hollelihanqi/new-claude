export type UpdateCheckErrorNotice = {
  title: string;
  message: string;
};

export const UPDATE_CHECK_OPTIONS = {
  headers: {
    "Cache-Control": "no-cache",
    Pragma: "no-cache",
  },
  timeout: 20_000,
};

function errorText(error: unknown): string {
  if (error instanceof Error && error.message.trim()) {
    return error.message.trim();
  }
  if (typeof error === "string" && error.trim()) {
    return error.trim();
  }
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}

/**
 * 更新检查失败与“没有新版本”是两个完全不同的状态。
 * 此函数只负责把真实错误转换为可操作提示，绝不能返回“已是最新”。
 */
export function describeUpdateCheckError(error: unknown): UpdateCheckErrorNotice {
  const raw = errorText(error);

  if (/fetch|network|request|timeout|connect|dns|tls|certificate|releases|404/i.test(raw)) {
    return {
      title: "检查更新失败",
      message: `无法连接更新服务器，请检查网络或代理后重试。技术信息：${raw}`,
    };
  }

  if (/platform|fallback|architecture|target/i.test(raw)) {
    return {
      title: "检查更新失败",
      message: `没有找到适用于当前系统的更新包。技术信息：${raw}`,
    };
  }

  if (/json|manifest|signature|format|parse/i.test(raw)) {
    return {
      title: "检查更新失败",
      message: `更新信息无效或签名校验失败。技术信息：${raw}`,
    };
  }

  return {
    title: "检查更新失败",
    message: raw || "发生未知错误，请稍后重试。",
  };
}
