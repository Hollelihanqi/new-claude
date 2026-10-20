import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { api } from "./api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue([]) }));

describe("模型检测 IPC 参数契约", () => {
  it("按修订号单独请求编辑配置，批量历史预览使用一个调用", async () => {
    const locator = { scope: "user" as const, name: "demo" };
    await api.readMcpServiceConfig(locator, "revision");
    expect(invoke).toHaveBeenLastCalledWith("read_mcp_service_config", { locator, expectedRevision: "revision" });
    await api.chatGptPreviewMany([]);
    expect(invoke).toHaveBeenLastCalledWith("chatgpt_preview_many", { requests: [] });
  });
  it("新建环境也传入后端必需的 env，编辑环境传入自己的 CA 范围", async () => {
    await api.detectModels("https://gateway.example.com", "test-only-key");
    expect(invoke).toHaveBeenLastCalledWith("detect_models", {
      env: "", baseUrl: "https://gateway.example.com", token: "test-only-key",
    });
    await api.detectModels("https://gateway.example.com", "test-only-key", "corp");
    expect(invoke).toHaveBeenLastCalledWith("detect_models", {
      env: "corp", baseUrl: "https://gateway.example.com", token: "test-only-key",
    });
  });
});
