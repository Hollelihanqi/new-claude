import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// 保留启动阶段的凭证访问边界，不再扫描界面说明文字。
describe("启动凭证访问边界", () => {
  it("应用启动不主动调用会解密凭证的完整健康检查", () => {
    expect(readFileSync(resolve("src/App.tsx"), "utf8")).not.toContain("startupHealthCheck");
  });
});
