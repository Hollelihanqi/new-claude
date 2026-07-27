import { describe, it, expect } from "vitest";
import {
  REDACTED,
  redactConfig,
  restoreRedactedValues,
  sensitivePathsForConfig,
  inferTransport,
  validateLocatorForScope,
  serviceSearchText,
  buildSyncTargetColumns,
  serviceContextLabel,
  syncTargetDisplayLabel,
} from "./mcpForm";
import type { McpService } from "../../api";

// RFC 6901 指针解码必须全量替换 ~1/~0；含多个 / 或 ~ 的敏感 key 不能脱敏遗漏。
describe("pointer decode (RFC 6901) — 多 / 与 ~ 的 key", () => {
  it("flat key TOKEN/a/b~c 正确脱敏并恢复，且不改原对象", () => {
    const config = { "TOKEN/a/b~c": "secret-value", safe: "x" } as Record<string, unknown>;
    const paths = ["/TOKEN~1a~1b~0c"];
    const redacted = redactConfig(config, paths);
    expect(redacted["TOKEN/a/b~c"]).toBe(REDACTED);
    expect(redacted.safe).toBe("x");
    // 原对象未被修改
    expect(config["TOKEN/a/b~c"]).toBe("secret-value");
    const restored = restoreRedactedValues(redacted, config, paths);
    expect(restored["TOKEN/a/b~c"]).toBe("secret-value");
  });

  it("嵌套 env 下含 / 和 ~ 的 key 正确脱敏并恢复", () => {
    const config = {
      env: { "API_TOKEN/x~y": "v", HARMLESS: "h" },
    } as unknown as Record<string, unknown>;
    const paths = ["/env/API_TOKEN~1x~0y"];
    const redacted = redactConfig(config, paths);
    expect((redacted.env as Record<string, unknown>)["API_TOKEN/x~y"]).toBe(REDACTED);
    expect((redacted.env as Record<string, unknown>).HARMLESS).toBe("h");
    const restored = restoreRedactedValues(redacted, config, paths);
    expect((restored.env as Record<string, unknown>)["API_TOKEN/x~y"]).toBe("v");
  });

  it("restore 保留用户已修改的值，只还原 REDACTED", () => {
    const original = { TOKEN: "old" } as Record<string, unknown>;
    const candidate = { TOKEN: "new" } as Record<string, unknown>;
    const restored = restoreRedactedValues(candidate, original, ["/TOKEN"]);
    expect(restored.TOKEN).toBe("new");
  });

  it("restore 在缺少 previousRaw 且仍为 REDACTED 时报错", () => {
    const candidate = { TOKEN: REDACTED } as Record<string, unknown>;
    expect(() => restoreRedactedValues(candidate, undefined, ["/TOKEN"])).toThrow();
  });

  it("新建服务或非允许路径不能提交脱敏占位符", () => {
    expect(() =>
      restoreRedactedValues({ TOKEN: REDACTED }, undefined, [])
    ).toThrow(/保留脱敏占位符/);
    expect(() =>
      restoreRedactedValues(
        { TOKEN: "old", safe: REDACTED },
        { TOKEN: "old", safe: "value" },
        ["/TOKEN"]
      )
    ).toThrow(/保留脱敏占位符/);
  });

  it("动态识别新输入的 env/header 凭据并正确转义路径", () => {
    const config = {
      env: { "API_TOKEN/x~y": "secret", HARMLESS: "ok" },
      headers: { Authorization: "Bearer value" },
    };
    const paths = sensitivePathsForConfig(config);
    expect(paths).toContain("/env/API_TOKEN~1x~0y");
    expect(paths).toContain("/headers/Authorization");
    const redacted = redactConfig(config, paths);
    expect((redacted.env as Record<string, unknown>)["API_TOKEN/x~y"]).toBe(REDACTED);
    expect((redacted.headers as Record<string, unknown>).Authorization).toBe(REDACTED);
    expect((redacted.env as Record<string, unknown>).HARMLESS).toBe("ok");
  });
});

describe("inferTransport", () => {
  it("streamable-http 视为 http", () => {
    expect(inferTransport({ type: "streamable-http", url: "https://a" })).toBe("http");
  });
  it("无 type 的 url 视为 unknown（不能误判 stdio）", () => {
    expect(inferTransport({ url: "https://a" })).toBe("unknown");
  });
  it("有 command 无 type 视为 stdio", () => {
    expect(inferTransport({ command: "node" })).toBe("stdio");
  });
});

describe("validateLocatorForScope", () => {
  it("user 不允许带 instance", () => {
    const e = validateLocatorForScope({ scope: "user", name: "n", instanceId: "a" });
    expect(e.instanceId).toBeTruthy();
  });
  it("local 必须同时有 instance 和 project", () => {
    const e = validateLocatorForScope({ scope: "local", name: "n" });
    expect(e.instanceId).toBeTruthy();
    expect(e.projectPath).toBeTruthy();
  });
  it("project 不能带 instance", () => {
    const e = validateLocatorForScope({ scope: "project", name: "n", instanceId: "a" });
    expect(e.instanceId).toBeTruthy();
  });
});

describe("serviceSearchText", () => {
  it("聚合名称、command、url、实例、项目（小写）", () => {
    const s = {
      locator: {
        scope: "local",
        name: "MyService",
        instanceId: "Alpha",
        projectPath: "C:\\proj",
      },
      config: { command: "node", url: "https://x" },
      transport: "stdio",
      effectiveState: "effective",
      enabled: true,
      shadowedBy: [],
      shadowedContextCount: 0,
      sourceId: "x",
      revision: "r",
      sensitivePaths: [],
      warnings: [],
    } as unknown as McpService;
    const t = serviceSearchText(s);
    expect(t).toContain("myservice");
    expect(t).toContain("node");
    expect(t).toContain("alpha");
  });
});

describe("MCP 列表展示模型", () => {
  it("默认保留 ChatGPT 列，未来目标按适配器新增独立列", () => {
    expect(buildSyncTargetColumns([])).toEqual([
      { targetId: "codex", label: "ChatGPT" },
    ]);
    expect(
      buildSyncTargetColumns([
        { targetId: "codex", targetLabel: "Codex" },
        { targetId: "workbuddy", targetLabel: "WorkBuddy" },
      ])
    ).toEqual([
      { targetId: "codex", label: "ChatGPT" },
      { targetId: "workbuddy", label: "WorkBuddy" },
    ]);
    expect(syncTargetDisplayLabel("codex", "Codex")).toBe("ChatGPT");
  });

  it("用户级范围不重复显示“全局”", () => {
    const service = {
      locator: { scope: "user", name: "demo" },
    } as McpService;
    expect(serviceContextLabel(service)).toBe("");
  });
});
