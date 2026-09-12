import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

// 界面文案不得写 Markdown 强调标记（`**粗体**`）。
//
// **这个坑本项目踩过两次**：Mantine 的 `<Text>` 不渲染 Markdown，星号会**原样显示**给用户
// （第一次是后果文案，改成了「」；第二次是 2026-09-12 这一轮改文案时又写回了 `**`，
// 直到把应用真跑起来看界面才发现）。
//
// 所以这里让它第三次不可能发生：扫描界面与后端返回的文案，禁止 `**`。
// 用「」表示强调 —— 与项目既有约定一致。

const SCAN_ROOTS = ["src", "src-tauri/src"];

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === "node_modules" || entry === "target" || entry === "dist") continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) walk(full, out);
    else if (/\.(tsx?|rs)$/.test(entry) && !/\.test\.tsx?$/.test(entry)) out.push(full);
  }
  return out;
}

/** 剥掉注释后仍含 `**` 的行。
 *
 *  - 块注释、整行注释直接去掉；
 *  - **行尾注释**也要去掉（`... // 说明 **加粗**`）；行尾 `//` 要求前面是空白或行首，
 *    这样 `https://` 这类不会被误当成注释起点。
 */
function markdownEmphasisHits(file: string): string[] {
  const text = readFileSync(file, "utf8")
    .replace(/\/\*[\s\S]*?\*\//g, "");
  const hits: string[] = [];
  text.split("\n").forEach((raw, index) => {
    // 先按行首丢掉纯注释行（`// …`、`* …` 续行）。只靠下面的行尾正则不够 ——
    // 缩进后的 `  // …` 与 JSDoc 续行都漏得过去（第一版就漏了 `api.ts` 两行）。
    const trimmed = raw.trim();
    if (trimmed.startsWith("//") || trimmed.startsWith("*") || trimmed.startsWith("/*")) return;
    const code = raw.replace(/(^|\s)\/\/.*$/, "");
    if (code.includes("**")) hits.push(`${file}:${index + 1}  ${code.trim().slice(0, 80)}`);
  });
  return hits;
}

describe("界面文案不写 Markdown", () => {
  it("扫描器本身有效（对已知坏样本能命中）—— 防空跑假绿", () => {
    const sample = '  <Text>这是**加粗**的字</Text>';
    const code = sample.replace(/(^|\s)\/\/.*$/, "");
    expect(code.includes("**")).toBe(true);
    // 注释里的不算
    const commented = "let x = 1; // **不跟随链接**";
    expect(commented.replace(/(^|\s)\/\/.*$/, "").includes("**")).toBe(false);
    // URL 里的 // 不能被当成注释起点
    const url = 'const u = "https://x/**y**";';
    expect(url.replace(/(^|\s)\/\/.*$/, "").includes("**")).toBe(true);
  });

  it("界面文案与后端返回的文案里没有 `**`（要强调请用「」）", () => {
    const files = SCAN_ROOTS.flatMap((root) => walk(resolve(process.cwd(), root)));
    expect(files.length, "扫到的文件太少，可能路径写错了").toBeGreaterThan(20);
    const hits = files.flatMap(markdownEmphasisHits);
    expect(
      hits,
      `界面文案里写了 Markdown 的 **强调**，Mantine 的 <Text> 不渲染、星号会原样显示给用户。\n改用「」：\n${hits.join("\n")}`
    ).toEqual([]);
  });
});
