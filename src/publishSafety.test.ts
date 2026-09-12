import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

// 公开前的泄漏回归护栏（对应 P0-A#5「清理内部 IP 与所有组织专属值」）。
//
// 本仓要公开，所以源码里不能出现组织内网地址、机器 SID、计算机名/用户名这类标识。
// 这类东西靠人工扫描容易漏（本轮就漏过：分析文档自身、以及本会话写进测试的
// 真实机器 SID 都是事后才发现的），所以钉成自动化检查。

// 扫**整个仓库的文本文件**，而不是只看源码目录。
// 教训：本轮的两次真实泄漏一次在 docs/、一次在 README，只扫 src 的护栏全程是绿的。
const SKIP_DIRS = new Set([
  "node_modules",
  "target",
  "dist",
  ".git",
  ".vite",
  "coverage",
  ".claude",
]);
// 只扫文本类文件；二进制（png/ico/pdf…）读成 utf8 只会得到乱码，没有意义
const TEXT_FILE = /\.(rs|ts|tsx|js|jsx|mjs|cjs|md|markdown|json|jsonc|ya?ml|toml|sh|bash|zsh|ps1|psm1|cmd|bat|html|css|scss|txt|cfg|ini|conf|env|example)$/i;
// 本文件自己含这些正则字面量，必须排除，否则永远命中自己
const SELF = /publishSafety\.test\.ts$/;

/** 私网 / 保留 IPv4。全部禁止——测试夹具改用 example.com 一类保留域名即可。 */
const PRIVATE_IPV4 =
  /\b(?:10\.\d{1,3}|172\.(?:1[6-9]|2\d|3[01])|192\.168)\.\d{1,3}\.\d{1,3}\b/;

/** 本机 SID 形态：S-1-5-21-<三段长数字>-<RID>。中间的机器标识是唯一且敏感的。 */
const MACHINE_SID = /\bS-1-5-21-\d{9,}-\d{9,}-\d{9,}-\d{3,}\b/;

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (SKIP_DIRS.has(entry)) continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      walk(full, out);
    } else if (TEXT_FILE.test(entry) && !SELF.test(full)) {
      out.push(full);
    }
  }
  return out;
}

describe("公开前无内部标识泄漏", () => {
  const files = walk(resolve(process.cwd()));

  it("扫到了待检查的文本文件", () => {
    // 防止 walk 因为路径写错变成空跑，那样测试会假绿。
    // 阈值按"整个仓库的文本文件"量级给，不是按源码目录。
    expect(files.length).toBeGreaterThan(60);
  });

  it("仓库文本文件里没有私网 IP", () => {
    const hits: string[] = [];
    for (const file of files) {
      readFileSync(file, "utf8")
        .split("\n")
        .forEach((line, index) => {
          const found = line.match(PRIVATE_IPV4);
          if (found) hits.push(`${file}:${index + 1} → ${found[0]}`);
        });
    }
    expect(hits, `发现私网 IP：\n${hits.join("\n")}`).toEqual([]);
  });

  it("仓库文本文件里没有真实机器 SID", () => {
    const hits: string[] = [];
    for (const file of files) {
      const found = readFileSync(file, "utf8").match(MACHINE_SID);
      if (found) hits.push(`${file} → ${found[0]}`);
    }
    expect(hits, `发现机器 SID：\n${hits.join("\n")}`).toEqual([]);
  });
});
