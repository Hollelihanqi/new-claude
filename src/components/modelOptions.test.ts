import { describe, expect, it } from "vitest";
import { PRESET_MODELS, buildModelOptions } from "./modelOptions";

describe("buildModelOptions", () => {
  it("检测成功后只显示当前网关的模型,不再混入公司网关预设", () => {
    const opts = buildModelOptions(["claude-opus-5", "claude-sonnet-5"]);
    expect(opts).toEqual(["claude-opus-5", "claude-sonnet-5"]);
    PRESET_MODELS.forEach((m) => expect(opts).not.toContain(m));
  });

  it("检测到三个模型时只返回三个候选", () => {
    const opts = buildModelOptions(
      ["gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.6-sol"]
    );
    expect(opts).toEqual(["gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.6-sol"]);
  });

  it("从未检测成功时回退预设兜底(检测不可用时仍可手动选择)", () => {
    expect(buildModelOptions([])).toEqual(PRESET_MODELS);
  });

  it("网关结果去重，清理空白并保留顺序", () => {
    expect(buildModelOptions(
      [" a ", "b", "a", " "]
    )).toEqual(["a", "b"]);
  });
});
