import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// 安全承诺回归护栏。
//
// 下面这些句子曾经出现在界面上，但与代码事实不符（依据见
// docs/凭证分域清单-2026-09-12.md §三）：
//   - Windows 上 gateway key 的 DPAPI 密文会进 config.json、config.backup.json，
//     并被 backup_config 原样复制到桌面（main.rs:1260-1274）；
//   - Windows 不使用系统凭据管理器，用的是 DPAPI 用户级加密，且 entropy 为空
//     （credentials.rs:101-111）⇒ 同登录用户下的其他程序也能解密；
//   - WorkBuddy 与 MCP 的密钥是明文文件，不在任何「钥匙串/DPAPI」承诺覆盖内。
//
// 钉住它们不许回来。若将来真的改了实现（例如把密钥移出配置备份），
// 应当同步改这条测试并在注释里写明依据，而不是直接删掉断言。
const FORBIDDEN = [
  "不进入诊断和配置备份",
  "只有你本人能解密",
  "仅你本人可解密",
  "不包含明文密钥的配置副本",
  "API Key 使用系统安全存储",
];

const CLAIM_SOURCES = [
  "src/components/SettingsPanel.tsx",
  "src/components/GuidePanel.tsx",
  "src/components/ConfigPanel.tsx",
  "src/components/WorkBuddyPanel.tsx",
];

const read = (file: string) => readFileSync(resolve(process.cwd(), file), "utf8");

describe("安全承诺与代码事实一致", () => {
  it("界面不再出现已被证伪的承诺", () => {
    for (const file of CLAIM_SOURCES) {
      const text = read(file);
      for (const phrase of FORBIDDEN) {
        expect(text, `${file} 又出现了已被证伪的说法：「${phrase}」`).not.toContain(phrase);
      }
    }
  });

  it("明说了 WorkBuddy / MCP 密钥是明文，以及 DPAPI 的作用域边界", () => {
    // 只消除错误承诺、不给替代说明，等于把用户从「被骗」换成「不知情」。
    // 所以正向也要钉住：设置页与引导页必须披露这两个事实。
    expect(read("src/components/SettingsPanel.tsx")).toContain("明文文件");
    const guide = read("src/components/GuidePanel.tsx");
    expect(guide).toContain("明文文件");
    expect(guide).toContain("同一登录用户下的其他程序");
  });

  it("应用启动不自动读取钥匙串凭证", () => {
    // 完整健康检查会解密每个网关 Key；若在 App 启动流程调用，就会在 macOS
    // 每次升级或未完成检测后弹系统授权框。钥匙串读取只能由用户主动操作触发。
    expect(read("src/App.tsx")).not.toContain("startupHealthCheck");
  });
});
