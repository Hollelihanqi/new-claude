import { describe, expect, it } from "vitest";
import { PRESET_MODELS, buildModelOptions } from "./modelOptions";

describe("buildModelOptions", () => {
  it("检测成功后只显示当前网关的模型,不再混入公司网关预设", () => {
    const opts = buildModelOptions(["claude-opus-5", "claude-sonnet-5"], []);
    expect(opts).toEqual(["claude-opus-5", "claude-sonnet-5"]);
    PRESET_MODELS.forEach((m) => expect(opts).not.toContain(m));
  });

  it("已保存的档位取值附加在末尾,防止既有选择从选项里消失", () => {
    const opts = buildModelOptions(
      ["claude-sonnet-5"],
      ["claude-opus-5", "", "  ", "claude-sonnet-5"]
    );
    expect(opts).toEqual(["claude-sonnet-5", "claude-opus-5"]);
  });

  it("从未检测成功时回退预设兜底(检测不可用时仍可手动选择)", () => {
    expect(buildModelOptions([], [])).toEqual(PRESET_MODELS);
  });
});
