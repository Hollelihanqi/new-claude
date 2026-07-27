// MCP 表单/脱敏/筛选纯函数。不持有状态、不直接读写文件。
import type {
  McpLocator,
  McpScope,
  McpService,
  McpSyncTargetInfo,
  McpTransport,
} from "../../api";

export const REDACTED = "__CC_MANAGER_REDACTED__";

export const SCOPE_LABELS: Record<McpScope, string> = {
  user: "用户级",
  local: "项目本地",
  project: "项目共享",
};

export const SCOPE_DESCRIPTIONS: Record<McpScope, string> = {
  user: "所有项目、所有 Claude 实例可用",
  local: "仅指定项目、指定 Claude 实例可用",
  project: "写入项目 .mcp.json，可由团队共享",
};

export const TRANSPORT_LABELS: Record<McpTransport, string> = {
  stdio: "本地进程",
  http: "HTTP",
  sse: "SSE（兼容旧服务）",
  ws: "WebSocket",
  unknown: "未知类型",
};

export const SCOPE_BADGE_COLOR: Record<McpScope, string> = {
  user: "blue",
  local: "orange",
  project: "cyan",
};

export interface SyncTargetColumn {
  targetId: string;
  label: string;
}

export function syncTargetDisplayLabel(targetId: string, targetLabel: string): string {
  return targetId === "codex" ? "ChatGPT" : targetLabel;
}

/** 目标适配器驱动列定义；保留 ChatGPT 空列表列，新增目标会自动新增独立列。 */
export function buildSyncTargetColumns(
  targets: Pick<McpSyncTargetInfo, "targetId" | "targetLabel">[]
): SyncTargetColumn[] {
  const columns = new Map<string, string>([["codex", "ChatGPT"]]);
  for (const target of targets) {
    if (!columns.has(target.targetId)) {
      columns.set(
        target.targetId,
        syncTargetDisplayLabel(target.targetId, target.targetLabel)
      );
    }
  }
  return [...columns].map(([targetId, label]) => ({ targetId, label }));
}

/** 用户级只显示“用户级”徽标，不再在徽标下重复显示“全局”。 */
export function serviceContextLabel(service: McpService): string {
  const { locator } = service;
  if (locator.scope === "user") return "";
  const project = locator.projectPath?.split(/[\\/]/).filter(Boolean).pop() ?? "未命名项目";
  if (locator.scope === "local") {
    return `${locator.instanceId ?? "默认实例"} / ${project}`;
  }
  return project;
}

const SENSITIVE_KEYS = new Set([
  "authorization",
  "proxy-authorization",
  "cookie",
  "set-cookie",
  "token",
  "access_token",
  "refresh_token",
  "secret",
  "client_secret",
  "password",
  "passwd",
  "api_key",
  "apikey",
  "credential",
]);
const SENSITIVE_SUBSTRINGS = [
  "TOKEN",
  "SECRET",
  "PASSWORD",
  "API_KEY",
  "APIKEY",
  "CREDENTIAL",
];

/** 仅用于 React key / 去重，不作为后端身份令牌。 */
export function locatorKey(locator: McpLocator): string {
  return [
    locator.scope,
    locator.instanceId ?? "",
    locator.projectPath ?? "",
    locator.name,
  ].join("|");
}

export function inferTransport(config: Record<string, unknown>): McpTransport {
  const t = typeof config.type === "string" ? config.type : "";
  if (t === "http" || t === "streamable-http") return "http";
  if (t === "sse") return "sse";
  if (t === "ws") return "ws";
  if (t === "stdio") return "stdio";
  if (typeof config.command === "string" && config.command.length > 0) return "stdio";
  return "unknown";
}

// ---------- JSON Pointer 脱敏 ----------

function decodePointerToken(value: string): string {
  // RFC 6901：必须全量替换，普通 String.replace 只替换首次出现的 ~1/~0，
  // 含多个 / 或 ~ 的 key 会定位失败导致脱敏遗漏。
  return value.replace(/~1/g, "/").replace(/~0/g, "~");
}

function encodePointerToken(value: string): string {
  return value.replace(/~/g, "~0").replace(/\//g, "~1");
}

function isSensitiveKey(key: string): boolean {
  if (SENSITIVE_KEYS.has(key.toLowerCase())) return true;
  const upper = key.toUpperCase();
  return SENSITIVE_SUBSTRINGS.some((part) => upper.includes(part));
}

/** 与 Rust 后端使用同一套敏感键规则，供用户新输入凭据后的即时 UI 脱敏。 */
export function sensitivePathsForConfig(config: Record<string, unknown>): string[] {
  const out: string[] = [];
  function walk(value: unknown, pointer: string) {
    if (Array.isArray(value)) {
      value.forEach((item, index) => walk(item, `${pointer}/${index}`));
      return;
    }
    if (!value || typeof value !== "object") return;
    for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
      const next = `${pointer}/${encodePointerToken(key)}`;
      if (isSensitiveKey(key)) {
        out.push(next);
      } else {
        walk(child, next);
      }
    }
  }
  walk(config, "");
  return out;
}

function pointerGet(root: unknown, pointer: string): unknown {
  if (pointer === "") return root;
  let cur: unknown = root;
  for (const raw of pointer.split("/")) {
    if (raw === "") continue;
    const token = decodePointerToken(raw);
    if (cur && typeof cur === "object" && !Array.isArray(cur)) {
      cur = (cur as Record<string, unknown>)[token];
    } else if (Array.isArray(cur)) {
      cur = cur[Number(token)];
    } else {
      return undefined;
    }
    if (cur === undefined) return undefined;
  }
  return cur;
}

function deepClone(v: unknown): unknown {
  if (typeof structuredClone === "function") return structuredClone(v);
  return JSON.parse(JSON.stringify(v));
}

function pointerSet(root: unknown, pointer: string, value: unknown): unknown {
  if (pointer === "") return value;
  const clone = deepClone(root);
  const parts = pointer
    .split("/")
    .map((p) => decodePointerToken(p))
    .filter((p, i) => !(p === "" && i === 0));
  let cur: Record<string, unknown> | unknown[] = clone as
    | Record<string, unknown>
    | unknown[];
  for (let i = 0; i < parts.length; i++) {
    const token = parts[i];
    const last = i === parts.length - 1;
    if (Array.isArray(cur)) {
      const idx = Number(token);
      if (last) {
        cur[idx] = value;
      } else {
        cur = cur[idx] as Record<string, unknown> | unknown[];
      }
    } else {
      const obj = cur as Record<string, unknown>;
      if (last) {
        obj[token] = value;
      } else {
        cur = obj[token] as Record<string, unknown> | unknown[];
      }
    }
  }
  return clone;
}

/** 深拷贝并把敏感位置替换为 REDACTED；不修改原对象。 */
export function redactConfig(
  config: Record<string, unknown>,
  sensitivePaths: string[]
): Record<string, unknown> {
  let out: unknown = deepClone(config);
  for (const p of sensitivePaths) {
    if (pointerGet(out, p) !== undefined) {
      out = pointerSet(out, p, REDACTED);
    }
  }
  return (out ?? {}) as Record<string, unknown>;
}

/**
 * 保存原始 JSON 时恢复脱敏值：candidate 里仍是 REDACTED 的位置，用 previousRaw 原值还原。
 * 用户显式改过的（非 REDACTED）保持不变。新建服务中出现 REDACTED 视为非法。
 */
export function restoreRedactedValues(
  candidate: Record<string, unknown>,
  previousRaw: Record<string, unknown> | undefined,
  sensitivePaths: string[]
): Record<string, unknown> {
  let out: unknown = deepClone(candidate);
  const allowed = new Set(sensitivePaths);
  const placeholders: string[] = [];
  function findPlaceholders(value: unknown, pointer: string) {
    if (value === REDACTED) {
      placeholders.push(pointer);
      return;
    }
    if (Array.isArray(value)) {
      value.forEach((item, index) => findPlaceholders(item, `${pointer}/${index}`));
      return;
    }
    if (!value || typeof value !== "object") return;
    for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
      findPlaceholders(child, `${pointer}/${encodePointerToken(key)}`);
    }
  }
  findPlaceholders(out, "");
  for (const pointer of placeholders) {
    if (!allowed.has(pointer)) {
      throw new Error(`保留脱敏占位符不能出现在 ${pointer || "配置根节点"}`);
    }
  }
  for (const p of sensitivePaths) {
    const cur = pointerGet(out, p);
    if (cur === REDACTED) {
      if (!previousRaw) {
        throw new Error(`敏感字段 ${p} 仍为占位符，请输入新值或移除`);
      }
      const original = pointerGet(previousRaw, p);
      if (original === undefined || original === REDACTED) {
        throw new Error(`敏感字段 ${p} 没有可恢复的原值，请输入新值或移除`);
      }
      out = pointerSet(out, p, original);
    }
  }
  return (out ?? {}) as Record<string, unknown>;
}

/** 校验 locator 与所选作用域是否匹配；返回字段名 -> 错误文案。 */
export function validateLocatorForScope(locator: McpLocator): Record<string, string> {
  const errs: Record<string, string> = {};
  if (locator.scope === "user") {
    if (locator.instanceId) errs.instanceId = "用户级不能指定实例";
    if (locator.projectPath) errs.projectPath = "用户级不能指定项目";
  } else if (locator.scope === "local") {
    if (!locator.instanceId) errs.instanceId = "项目本地必须选择实例";
    if (!locator.projectPath) errs.projectPath = "项目本地必须选择项目";
  } else if (locator.scope === "project") {
    if (locator.instanceId) errs.instanceId = "项目共享不能指定实例";
    if (!locator.projectPath) errs.projectPath = "项目共享必须选择项目";
  }
  return errs;
}

/** 筛选/搜索用的可读文本：名称、command、url、项目路径、实例名。 */
export function serviceSearchText(service: McpService): string {
  const parts = [
    service.locator.name,
    typeof service.config.command === "string" ? service.config.command : "",
    typeof service.config.url === "string" ? service.config.url : "",
    service.locator.projectPath ?? "",
    service.locator.instanceId ?? "",
    service.transport,
  ];
  return parts.join(" ").toLowerCase();
}
