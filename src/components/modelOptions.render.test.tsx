import { renderToString } from "react-dom/server";
import { Autocomplete, MantineProvider } from "@mantine/core";
import { describe, expect, it } from "vitest";
import { buildModelOptions } from "./modelOptions";

describe("检测结果渲染", () => {
  it.each([
    ["两个档位保留相同的旧模型", ["gateway-model"], ["claude-opus-5", "claude-opus-5-thinking", "claude-opus-5"]],
    ["网关返回重复模型", ["gateway-model", "gateway-model", " gateway-model "], ["gateway-model"]],
  ])("%s 时页面仍能渲染", (_label, detected, saved) => {
    expect(() => renderToString(
      <MantineProvider>
        <Autocomplete label="Opus 档" value={saved[0]} data={buildModelOptions(detected)} />
      </MantineProvider>
    )).not.toThrow();
  });
});
