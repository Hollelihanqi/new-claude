import { describe, expect, it } from "vitest";
import { describeUpdateCheckError, UPDATE_CHECK_OPTIONS } from "./updateCheck";

describe("describeUpdateCheckError", () => {
  it("网络错误必须显示检查失败，不能伪装成已是最新", () => {
    const notice = describeUpdateCheckError(new Error("network request timed out"));

    expect(notice.title).toBe("检查更新失败");
    expect(notice.message).toContain("无法连接更新服务器");
    expect(`${notice.title}${notice.message}`).not.toContain("已是最新");
  });

  it("平台不匹配必须明确提示没有对应安装包", () => {
    const notice = describeUpdateCheckError("platform darwin-aarch64 not found");

    expect(notice.title).toBe("检查更新失败");
    expect(notice.message).toContain("适用于当前系统的更新包");
  });

  it("保留未知错误，便于继续诊断", () => {
    const notice = describeUpdateCheckError("unexpected updater failure");

    expect(notice.title).toBe("检查更新失败");
    expect(notice.message).toBe("unexpected updater failure");
  });

  it("手动检查必须绕过缓存并设置超时", () => {
    expect(UPDATE_CHECK_OPTIONS.headers["Cache-Control"]).toBe("no-cache");
    expect(UPDATE_CHECK_OPTIONS.headers.Pragma).toBe("no-cache");
    expect(UPDATE_CHECK_OPTIONS.timeout).toBeGreaterThan(0);
  });
});
