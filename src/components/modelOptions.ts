// 文档里列出的常用模型别名:仅在当前网关从未检测到模型时作为下拉兜底,
// 保证检测不可用(如网关线路不稳)时仍可手动选择。检测成功后一律以网关
// 实际返回的模型为准,避免第三方网关误选公司网关专用的预设名。
export const PRESET_MODELS = [
  "claude-sonnet-4-6",
  "claude-opus-4-7",
  "claude-haiku-4-5",
  "deepseek-v4-pro",
  "deepseek-v4-flash",
  "glm-5.2",
  "glm-5.1",
  "glm-5",
  "glm-5-turbo",
  "claude-zhipu-5.2",
  "kimi-k2.7-code",
  "kimi-k2.6",
  "minimax-m2.7",
  "qwen3.7-max",
  "qwen3.7-plus",
  "qwen3.6-flash",
  "claude-qw3.7-max",
  "claude-qw3.6-plus",
];

/**
 * 模型下拉候选:检测成功(detected 非空)→ 只显示当前网关的可用模型,
 * 并附带表单已保存的档位取值(防既有选择从选项里凭空消失);
 * 从未检测成功 → 预设兜底。输入做 trim/去空,结果去重且保持顺序稳定。
 */
export function buildModelOptions(
  detected: string[],
  savedModels: string[]
): string[] {
  const clean = (arr: string[]) =>
    arr.map((m) => m.trim()).filter((m) => m.length > 0);

  if (detected.length > 0) {
    const list = clean(detected);
    const seen = new Set(list);
    const extras = clean(savedModels).filter((m) => !seen.has(m));
    return [...list, ...extras];
  }
  return PRESET_MODELS;
}
