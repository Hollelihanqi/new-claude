import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// 保留安装升级和历史数据兼容契约，不测试界面品牌文案。
describe("安装与存量数据兼容", () => {
  const config = JSON.parse(readFileSync(resolve("src-tauri/tauri.conf.json"), "utf8"));
  it("Windows 安装包保持 MSI 升级码，避免重复安装", () => {
    expect(config.bundle.windows.wix.upgradeCode).toBe("3f2eb614-3df5-5951-9163-e54ae1820787");
  });
  it("macOS 保持应用标识", () => {
    expect(config.identifier).toBe("com.pathmux.desktop");
  });
  it("保留存量配置与环境历史目录", () => {
    const source = readFileSync(resolve("src-tauri/src/main.rs"), "utf8");
    expect(source).toContain('.join(".cc-manager")');
    expect(source).toContain('.join(".claude-split")');
  });
});
