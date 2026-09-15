import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

// 品牌标识一致性护栏（P0-C#14）。
//
// 现状梳理（**不要一刀切**，这三类性质不同）：
//
//   | 出现 | 性质 | 处理 |
//   |---|---|---|
//   | 界面里的产品名 | 用户可见文案 | **必须统一**为「PathMux」 |
//   | `PathMux` | `tauri.conf.json` 的 productName（装机产物/安装目录/窗口名） | 与界面产品名保持一致 |
//   | `~/.cc-manager` | 真实配置目录名 | **保留**，改了会断掉存量用户配置 |
//   | `~/.claude-split` | 环境数据目录（含会话历史） | **保留**，改它等于搬走用户历史 |
//   | `com.pathmux.desktop` | 应用标识（identifier） | 已随品牌改名（用户裁定；macOS 侧只有本人装过、可重装） |
//   | `cc-switch` | 对**第三方项目**口径的引用（用量统计同口径） | 保留，那是署名不是自我命名 |
//
// 所以这里只钉「界面产品名统一、且旧名不许回来」这两条，不是禁掉全部字符串。

/// 改名前的旧名。**不许出现在界面文案里** —— 它们是这次手术要切掉的东西。
const RETIRED_NAMES = ["Claude 管理中心", "Claude Center", "并路 PathMux"];

const SCAN_ROOTS = ["src", "src-tauri/src"];

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === "node_modules") continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      walk(full, out);
    } else if (/\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry)) {
      out.push(full);
    }
  }
  return out;
}

describe("品牌标识一致", () => {
  it("界面文案里不出现旧产品名（改名要切干净，旧名不许回来）", () => {
    const hits: string[] = [];
    // 注释里需要解释这个决定，所以先剥掉注释再查。
    // 块注释要跨行剥（`/* … */`）—— 上一版只处理单行，结果命中了自己的 JSX 注释 `{/* … */}`。
    const stripComments = (text: string) =>
      text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^[ \t]*\/\/.*$/gm, "");
    for (const file of SCAN_ROOTS.flatMap((root) => walk(resolve(process.cwd(), root)))) {
      stripComments(readFileSync(file, "utf8"))
        .split("\n")
        .forEach((line, index) => {
          for (const retired of RETIRED_NAMES) {
            if (line.includes(retired)) hits.push(`${file}:${index + 1} — ${retired}`);
          }
        });
    }
    expect(
      hits,
      `界面里还留着改名前后的旧产品名（应统一为「PathMux」）：\n${hits.join("\n")}`
    ).toEqual([]);
  });

  it("tauri 的 productName 与升级码（改名不能断掉 Windows 升级路径）", () => {
    const conf = JSON.parse(
      readFileSync(resolve(process.cwd(), "src-tauri/tauri.conf.json"), "utf8")
    );
    expect(conf.productName).toBe("PathMux");
    // **改名时最容易漏、后果最坏的一条**：Tauri 默认由 productName 推导 MSI 升级码
    // （`Uuid::new_v5(DNS, "<productName>.exe.app.x64")`）。改了 productName 的
    // 同时不显式固定 upgradeCode，Windows 就会把新版本当成**另一个应用**：
    // 老版本不被替换、两个版本并存。这里钉住"必须显式固定且值不变"。
    expect(conf.bundle?.windows?.wix?.upgradeCode).toBe(
      "3f2eb614-3df5-5951-9163-e54ae1820787"
    );
  });

  it("应用标识已随品牌改名（且不再回退到旧标识）", () => {
    const conf = JSON.parse(
      readFileSync(resolve(process.cwd(), "src-tauri/tauri.conf.json"), "utf8")
    );
    // 2026-09-12 用户裁定：一并改掉。macOS 侧只有本人装过、可卸载重装，
    // 所以"新标识会被当成另一个应用"这个代价可接受。
    expect(conf.identifier).toBe("com.pathmux.desktop");
  });

  it("两个数据目录保持不变（改它们要搬用户数据，收益只是名字好看）", () => {
    const src = readFileSync(resolve(process.cwd(), "src-tauri/src/main.rs"), "utf8");
    expect(src).toContain('.join(".cc-manager")');
    expect(src).toContain('.join(".claude-split")');
  });
});

describe("术语统一（P1-5）", () => {
  it("不再使用已被取代的旧术语", () => {
    // 依据 docs/术语表-2026-09-12.md。标准词是：
    //   环境 / 网关环境 / 独立登录环境 / 默认 Claude
    //
    // 注意「单实例」（single-instance 应用）里的"实例"是**另一个意思**，必须先剔除。
    //
    // ⚠️ 这里曾经出过事故：一次 `空间→环境` 的批量改名把**本文件的字面量也改了**，
    //    于是断言从"禁止空间"变成"禁止环境"，全部命中。**改文案时不要在 src/ 里盲替换。**
    const FORBIDDEN = ["实例", "空间", "路由环境", "账户环境", "主账户"];
    const hits: string[] = [];
    const stripComments = (text: string) =>
      text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^[ \t]*\/\/.*$/gm, "");
    for (const file of SCAN_ROOTS.flatMap((root) => walk(resolve(process.cwd(), root)))) {
      stripComments(readFileSync(file, "utf8"))
        .split("\n")
        .forEach((line, index) => {
          const cleaned = line.split("单实例").join("");
          if (FORBIDDEN.some((word) => cleaned.includes(word))) {
            hits.push(`${file}:${index + 1}`);
          }
        });
    }
    expect(
      hits,
      `仍有旧术语（应统一为「环境 / 网关环境 / 独立登录环境 / 默认 Claude」）：\n${hits.join("\n")}`
    ).toEqual([]);
  });
});

describe("主账户不是环境（P1-5）", () => {
  it("不把主账户说成「环境」", () => {
    // 主账户**不是环境的一种**：它是"没切环境"时的那个账户，本应用只读不写。
    // 详见 docs/术语表-2026-09-12.md。这条曾被写错成「默认 Claude 环境」。
    //
    // 和其他护栏一样：先剥掉注释再查 —— 否则解释这条规则的注释自己就会命中
    // （已经踩过一次：本文件的注释里就写着「默认 Claude 环境」）。
    const hits: string[] = [];
    const stripComments = (text: string) =>
      text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^[ \t]*\/\/.*$/gm, "");
    for (const file of SCAN_ROOTS.flatMap((root) => walk(resolve(process.cwd(), root)))) {
      stripComments(readFileSync(file, "utf8"))
        .split("\n")
        .forEach((line, index) => {
          if (/默认\s*(Claude\s*)?环境/.test(line)) hits.push(`${file}:${index + 1}`);
        });
    }
    expect(hits, `把主账户写成了环境：\n${hits.join("\n")}`).toEqual([]);
  });
});
