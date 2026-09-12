import { describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { api } from "./api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue([]) }));

describe("模型检测 IPC 参数契约", () => {
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
