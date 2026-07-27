# MCP 服务管理新功能开发方案

> 本文档由 `feature-plan-doc` skill 产出。仅描述改动，不含已落地代码；落地由人确认后执行。

| 方案模型 | 作者 | 创建日期 | 完成日期 | 状态 |
| --- | --- | --- | --- | --- |
| GPT-5 | lihanqi | 2026-07-24 | —（审核通过后填写） | 待修正 |

## 1. 背景与目标

### 1.1 一句话需求

在 Claude 管理中心新增独立一级菜单「MCP 服务」，提供跨作用域、可识别来源、可安全启停、可编辑、可导入、可做基础连接检查的 MCP 服务管理能力，并与现有多实例同步机制正确协作。

### 1.2 已确认的设计选择

1. 「MCP 服务」是独立一级菜单，位于「空间」与「扩展」之间，不作为扩展中心的子页。
2. 管理三个 Claude Code 官方可编辑作用域，并在列表中明确显示作用域标记：
   - `user`：用户级；所有项目可用；存储在每个 Claude 实例的 `.claude.json` 顶层 `mcpServers`。
   - `local`：项目本地级；仅指定 Claude 实例、指定项目可用；存储在该实例 `.claude.json` 的 `projects[projectPath].mcpServers`。
   - `project`：项目共享级；项目内所有用户可见；存储在项目根目录 `.mcp.json`。
3. 同名服务不合并字段，按 Claude Code 的优先级显示覆盖关系：`local > project > user`。列表必须分别展示每个定义，禁止把同名服务折叠成一条后丢失来源信息。
4. 提供启停：
   - `user` 停用：从启用配置移入 `~/.cc-manager/mcp-disabled.json`，再通过现有三方同步从所有 Claude 实例移除。
   - `local` 停用：从指定实例、指定项目的本地配置移入 `~/.cc-manager/mcp-disabled.json`，不影响其他实例或项目。
   - `project` 停用：不删除团队共享的 `.mcp.json` 条目；把服务名写入该项目 `.claude/settings.local.json` 的 `disabledMcpjsonServers`，实现仅当前机器停用。
   - 页面字段统一命名为「配置状态」，禁止命名为「运行状态」；Project 开关文案为「本机启用/本机停用」。
5. 首期连接测试只做配置结构校验和基础可达性检查，不执行 MCP `initialize`、`tools/list` 或真实工具调用。
6. 凭据保持原 MCP 配置的存储方式，不迁移到系统凭据库；界面、预览、错误、通知和日志必须脱敏。
7. 页面采用「状态摘要 + 筛选工具栏 + 服务定义列表 + 大尺寸编辑抽屉」。
8. 支持新建、编辑、移动作用域、重命名、复制、启停、删除、粘贴单个服务 JSON、粘贴完整 `mcpServers`。
9. 原生表单支持 `stdio`、`http`/`streamable-http`、`sse`、`ws`；未知 transport 以「未知类型」标记，只允许通过原始 JSON 编辑，保存时不得丢失未知字段。
10. HTTP 是远程服务的默认推荐类型；SSE 在界面标记「兼容旧服务」；`streamable-http` 作为 HTTP 别名展示但原始值原样保存。
11. 不修改现有网关 Token 的加密、解密、清理和脚本注入路径。

### 1.3 官方配置语义基线

落地时以 Claude Code 官方 MCP 文档为配置语义基线：

- 作用域、存储位置和优先级：<https://code.claude.com/docs/en/mcp#mcp-installation-scopes>
- MCP 配置字段和 transport：<https://code.claude.com/docs/en/mcp#installing-mcp-servers>
- 项目 MCP 批准/拒绝设置：<https://code.claude.com/docs/en/settings>

本功能不调用 `claude mcp add/remove` 作为写入手段。所有配置读写由 Rust 后端直接完成，以便复用现有备份、原子写入、revision 检查和多实例同步。

### 1.4 功能边界定义

| 来源 | 是否展示 | 是否编辑 | 是否启停 | 说明 |
| --- | --- | --- | --- | --- |
| User | 是 | 是 | 是 | 跨项目、跨实例共享 |
| Local | 是 | 是 | 是 | 必须同时选定实例与项目 |
| Project | 是 | 是 | 是 | 编辑会修改项目根目录 `.mcp.json` |
| Managed MCP | 否 | 否 | 否 | 企业管理员控制，首期不扫描 |
| Plugin MCP | 否 | 否 | 否 | 由插件安装/启用状态控制 |
| claude.ai Connector | 否 | 否 | 否 | 不属于本地文件配置 |

## 2. 现状链路

### 2.1 完整调用链

| 步骤 | 位置 | 当前行为 | 新功能介入点 |
| --- | --- | --- | --- |
| 1. 一级页面类型 | `src/App.tsx:35` | `ViewId` 没有 MCP | 增加 `mcp` |
| 2. 侧边栏菜单 | `src/App.tsx:41-47` | 只有空间、扩展、洞察、诊断、设置 | 在空间后插入「MCP 服务」 |
| 3. 页面标题 | `src/App.tsx:49-56` | 没有 MCP 标题 | 增加「MCP 服务管理」 |
| 4. 页面渲染 | `src/App.tsx:304-325` | 根据本地 `view` 状态渲染五个业务页 | 渲染新增 `McpPanel` |
| 5. 扩展概览请求 | `src/components/ExtensionsPanel.tsx:19-24` | 调 `extensionOverview()` | 调用保持；返回值不再包含 MCP |
| 6. 扩展 MCP 卡片 | `src/components/ExtensionsPanel.tsx:7-13,37-54` | MCP 与 Skills/Plugins/Agents 混在网格中只读展示 | 删除 MCP 类型和卡片 |
| 7. 前端 IPC | `src/api.ts:36-41,95-110` | MCP 只有 `ExtensionGroup.items: string[]` | 新增完整 MCP 类型和命令 |
| 8. 后端扩展概览 | `src-tauri/src/main.rs:913-948` | 读取主账户 `~/.claude.json` 顶层 `mcpServers` 名称 | 删除这段 MCP 拼接 |
| 9. GUI 启动同步 | `src/App.tsx:192-203` | 启动即调用 `api.syncAll()` | 行为保持 |
| 10. 后端同步入口 | `src-tauri/src/main.rs:856-863` | 安装集成后调用 `sync::sync_configs()` | User 作用域保存后复用相同同步 |
| 11. CLI 前后同步 | `src-tauri/src/main.rs:1435-1452` | `--sync` 模式在 Claude 启动/退出时同步 | 行为保持 |
| 12. 配置三方合并 | `src-tauri/src/sync.rs:289-394` | 对顶层 `mcpServers` 和 `enabledPlugins` 做快照合并 | User 写入必须与此逻辑共用锁 |
| 13. 同步文件范围 | `src-tauri/src/sync.rs:445-463` | 主账户和全部实例 `.claude.json` 顶层字段 | Local/Project 不进入此合并 |
| 14. revision 既有范式 | `src-tauri/src/main.rs:570-577,660-677` | 用 mtime 拒绝覆盖后台改过的 settings 文件 | MCP 扩展为 `mtime+size+contentHash` revision |
| 15. 原子 JSON 写入 | `src-tauri/src/sync.rs:276-281` | 临时文件后 rename | 新 MCP 存储层增加备份和 Windows 可替换写入 |
| 16. 凭据存储 | `src-tauri/src/main.rs:689-735` | 仅 Profile Token 使用平台原生存储 | 明确不复用、不修改 |

### 2.2 关键现状代码

当前一级菜单没有 MCP：

```tsx
// src/App.tsx:35
type ViewId = "environment" | "extensions" | "insights" | "diagnostics" | "settings" | "guide";
```

当前扩展概览只返回 MCP 名称：

```rust
// src-tauri/src/main.rs:931-947
let claude_json = home().join(".claude.json");
let mcp_items = fs::read_to_string(&claude_json)
    .ok()
    .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
    .and_then(|value| value.get("mcpServers").and_then(|v| v.as_object()).cloned())
    .map(|map| {
        let mut names = map.keys().cloned().collect::<Vec<_>>();
        names.sort();
        names
    })
    .unwrap_or_default();
groups.push(ExtensionGroup {
    kind: "mcp".into(),
    label: "MCP Servers".into(),
    path: claude_json.display().to_string(),
    items: mcp_items,
});
```

当前 User MCP 会跨实例合并：

```rust
// src-tauri/src/sync.rs:445-463
let mut mcp_files: Vec<(String, PathBuf)> =
    vec![("__main__".into(), crate::home().join(".claude.json"))];
for n in names {
    if !n.is_empty() {
        mcp_files.push((n.clone(), instance_dir(n).join(".claude.json")));
    }
}
let (dsnap, written) = sync_domain("mcpServers", &mcp_files, snap.domains.get("mcpServers"));
```

### 2.3 必须补齐的跨流程上下文

当前 MCP 只透传「名称」。新功能必须在前后端完整透传：

1. `scope`：User、Local、Project。
2. `instanceId`：Local 必填；User/Project 为空。
3. `projectPath`：Local/Project 必填；User 为空。
4. `sourceId + revision`：用于预览后保存时检测外部修改。
5. `rawConfig`：保留未知 MCP 字段，禁止表单重建导致字段丢失。
6. `enabled`：不同作用域采用不同停用机制。
7. `effectiveState + shadowedBy`：标记同名配置的优先级结果。
8. `sensitivePaths`：标记需要在 UI 和预览中脱敏的 JSON Pointer。

缺少任意一项都会产生错误行为：写错配置文件、覆盖错误实例、把项目配置当用户配置同步、同名覆盖关系显示错误，或在预览中泄露凭据。

## 3. 方案总览

新增前端一级页面和后端独立 `mcp` 深模块。前端不直接读写文件，只提交带作用域定位和 revision 的领域动作；后端统一完成作用域发现、路径校验、配置无损读写、停用语义、覆盖关系计算、脱敏预览、基础测试和 User 作用域同步。

后端采用 `src-tauri/src/mcp/` 目录，不继续把逻辑堆入已超过 1500 行的 `main.rs`。存储层与校验层分离：

- `mcp/mod.rs`：领域类型、Tauri 命令、动作编排。
- `mcp/storage.rs`：作用域发现、路径解析、revision、备份、原子写入、停用仓库。
- `mcp/validation.rs`：配置校验、transport 推断、敏感字段识别、基础可达性测试。

前端采用 `src/components/mcp/` 目录：

- `McpPanel.tsx`：页面状态与查询编排。
- `McpServiceDrawer.tsx`：新增、编辑、复制、原始 JSON。
- `McpImportModal.tsx`：粘贴和批量导入预览。
- `mcpForm.ts`：表单/JSON 转换、UI 脱敏、筛选辅助。

## 4. 详细改动（逐文件逐处）

### 4.1 安装目录选择插件

#### 4.1.1 `package.json`、`pnpm-lock.yaml`、`src-tauri/Cargo.toml`

定位：`package.json:13-27`、`src-tauri/Cargo.toml:10-20`。

Before：工程未安装 Tauri Dialog 插件。

执行以下唯一安装命令，不手工编辑 lockfile：

```powershell
pnpm tauri add dialog
```

After 必须满足：

```json
// package.json dependencies 内存在
"@tauri-apps/plugin-dialog": "^2"
```

```toml
# src-tauri/Cargo.toml [dependencies] 内存在
tauri-plugin-dialog = "2"
```

`pnpm-lock.yaml` 和 `src-tauri/Cargo.lock` 由命令生成，禁止手写版本或删除其他依赖锁定。

#### 4.1.2 `src-tauri/capabilities/default.json`

定位：`src-tauri/capabilities/default.json:1-7`。

Before：

```json
"permissions": ["core:default", "updater:default", "process:default"]
```

After：

```json
"permissions": [
  "core:default",
  "updater:default",
  "process:default",
  "dialog:allow-open"
]
```

只开放目录选择所需的 `allow-open`，不增加文件系统插件权限；项目文件仍由受控 Rust 命令读写。

### 4.2 `src/App.tsx`：接入独立一级菜单

定位：`src/App.tsx:8-18,28-35,41-56,304-320`。

#### 4.2.1 import

Before：

```tsx
import {
  IconLayoutDashboard,
  IconChartLine,
  IconCircleCheck,
  IconAlertTriangle,
  IconChevronRight,
  IconStack2,
  IconStethoscope,
  IconSettings,
  IconHelpCircle,
} from "@tabler/icons-react";
import ExtensionsPanel from "./components/ExtensionsPanel";
```

After：

```tsx
import {
  IconLayoutDashboard,
  IconChartLine,
  IconCircleCheck,
  IconAlertTriangle,
  IconChevronRight,
  IconServerCog,
  IconStack2,
  IconStethoscope,
  IconSettings,
  IconHelpCircle,
} from "@tabler/icons-react";
import ExtensionsPanel from "./components/ExtensionsPanel";
import McpPanel from "./components/mcp/McpPanel";
```

`IconServerCog` 已存在于当前 `@tabler/icons-react` 依赖。

#### 4.2.2 ViewId、导航和标题

Before：

```tsx
type ViewId = "environment" | "extensions" | "insights" | "diagnostics" | "settings" | "guide";

const NAV = [
  { id: "environment", label: "空间", desc: "实例、网关与模型", icon: IconLayoutDashboard },
  { id: "extensions", label: "扩展", desc: "Skills、MCP 与 Agents", icon: IconStack2 },
];
```

After：

```tsx
type ViewId = "environment" | "mcp" | "extensions" | "insights" | "diagnostics" | "settings" | "guide";

const NAV: { id: ViewId; label: string; desc: string; icon: typeof IconLayoutDashboard }[] = [
  { id: "environment", label: "空间", desc: "实例、网关与模型", icon: IconLayoutDashboard },
  { id: "mcp", label: "MCP 服务", desc: "配置、作用域与测试", icon: IconServerCog },
  { id: "extensions", label: "扩展", desc: "Skills、Plugins 与 Agents", icon: IconStack2 },
  { id: "insights", label: "洞察", desc: "用量、模型与趋势", icon: IconChartLine },
  { id: "diagnostics", label: "诊断", desc: "检查、日志与修复", icon: IconStethoscope },
  { id: "settings", label: "设置", desc: "更新、证书与安全", icon: IconSettings },
];

const VIEW_TITLES: Record<ViewId, string> = {
  environment: "空间管理",
  mcp: "MCP 服务管理",
  extensions: "扩展中心",
  insights: "用量洞察",
  diagnostics: "诊断中心",
  settings: "系统设置",
  guide: "使用帮助",
};
```

#### 4.2.3 页面渲染

Before：

```tsx
{view === "environment" && <ConfigPanel onChanged={refreshEnv} env={env} usageData={usageData} />}
{view === "extensions" && <ExtensionsPanel />}
```

After：

```tsx
{view === "environment" && <ConfigPanel onChanged={refreshEnv} env={env} usageData={usageData} />}
{view === "mcp" && <McpPanel />}
{view === "extensions" && <ExtensionsPanel />}
```

不引入 React Router；沿用现有单窗口 `view` 状态切页机制。

### 4.3 `src/components/ExtensionsPanel.tsx`：移除重复 MCP 入口

定位：`src/components/ExtensionsPanel.tsx:2-13,30-35`。

Before：

```tsx
import { IconBrain, IconCommand, IconPlugConnected, IconRefresh, IconRobot, IconTool } from "@tabler/icons-react";

const ICONS = {
  skills: IconBrain,
  plugins: IconPlugConnected,
  agents: IconRobot,
  commands: IconCommand,
  mcp: IconTool,
};
```

After：

```tsx
import { IconBrain, IconCommand, IconPlugConnected, IconRefresh, IconRobot, IconTool } from "@tabler/icons-react";

const ICONS = {
  skills: IconBrain,
  plugins: IconPlugConnected,
  agents: IconRobot,
  commands: IconCommand,
};
```

页面说明改为：

```tsx
<div>
  <Title order={3}>扩展中心</Title>
  <Text size="sm" c="dimmed">统一查看主账户共享给所有实例的 Skills、Plugins、Agents 与 Commands。</Text>
</div>
```

不保留 MCP 摘要卡或跳转卡，防止用户误认为扩展中心和一级菜单是两个不同管理入口。

### 4.4 `src/api.ts`：增加完整 MCP 领域类型和 IPC

定位：`src/api.ts:36-41,95-133`。

#### 4.4.1 修改 ExtensionGroup

Before：

```ts
export interface ExtensionGroup {
  kind: "skills" | "plugins" | "agents" | "commands" | "mcp";
  label: string;
  path: string;
  items: string[];
}
```

After：

```ts
export interface ExtensionGroup {
  kind: "skills" | "plugins" | "agents" | "commands";
  label: string;
  path: string;
  items: string[];
}
```

#### 4.4.2 新增类型

在 `ExtensionGroup` 后完整新增：

```ts
export type McpScope = "user" | "local" | "project";
export type McpTransport = "stdio" | "http" | "sse" | "ws" | "unknown";
export type McpEffectiveState = "effective" | "partially-shadowed" | "shadowed" | "disabled";

export interface McpInstanceRef {
  id: string; // 主账户固定为 "__main__"
  label: string;
}

export interface McpProjectRef {
  path: string; // 后端 canonicalize 后的绝对路径
  label: string; // 目录名；重名时 UI 同时显示父路径
  discovered: boolean;
}

export interface McpLocator {
  scope: McpScope;
  name: string;
  instanceId?: string;
  projectPath?: string;
}

export interface McpShadowRef {
  scope: McpScope;
  name: string;
  instanceId?: string;
  projectPath?: string;
}

export interface McpService {
  locator: McpLocator;
  transport: McpTransport;
  rawTransport?: string;
  config: Record<string, unknown>;
  enabled: boolean;
  effectiveState: McpEffectiveState;
  shadowedBy: McpShadowRef[];
  shadowedContextCount: number;
  sourceId: string;
  revision: string;
  sensitivePaths: string[]; // RFC 6901 JSON Pointer
  warnings: string[];
}

export interface McpSourceIssue {
  sourceId: string;
  path: string;
  detail: string;
}

export interface McpState {
  services: McpService[];
  instances: McpInstanceRef[];
  projects: McpProjectRef[];
  revisions: Record<string, string>; // sourceId -> revision
  issues: McpSourceIssue[];
  summary: {
    total: number;
    enabled: number;
    disabled: number;
    warnings: number;
    shadowed: number;
  };
}

export interface McpSaveItem {
  target: McpLocator;
  config: Record<string, unknown>;
  overwrite: boolean;
}

export type McpChangeAction =
  | {
      op: "save";
      original?: McpLocator;
      target: McpLocator;
      config: Record<string, unknown>;
      overwrite?: boolean; // 仅导入同名覆盖时为 true；其他入口固定为 false
    }
  | {
      op: "batchSave";
      items: McpSaveItem[];
    }
  | {
      op: "setEnabled";
      target: McpLocator;
      enabled: boolean;
    }
  | {
      op: "delete";
      target: McpLocator;
    };

export interface McpChangeRequest {
  action: McpChangeAction;
  expectedRevisions: Record<string, string>;
}

export interface McpChangePreview {
  actionLabel: string;
  affectedSources: Array<{
    sourceId: string;
    path: string;
    scope: McpScope;
  }>;
  affectedInstances: string[];
  redactedBefore?: Record<string, unknown>;
  redactedAfter?: Record<string, unknown>;
  warnings: string[];
  expectedRevisions: Record<string, string>;
}

export interface McpTestRequest {
  locator?: McpLocator;
  name: string;
  config: Record<string, unknown>;
}

export interface McpTestStage {
  id: "schema" | "command" | "url" | "endpoint";
  status: "ok" | "warn" | "fail" | "skipped";
  detail: string;
}

export interface McpTestResult {
  ok: boolean;
  transport: McpTransport;
  stages: McpTestStage[];
  sanitizedDetail: string;
}
```

#### 4.4.3 新增 API

在 `api` 对象的 `extensionOverview` 后新增：

```ts
listMcpServices: (): Promise<McpState> =>
  invoke("list_mcp_services"),
registerMcpProject: (path: string): Promise<McpState> =>
  invoke("register_mcp_project", { path }),
unregisterMcpProject: (path: string): Promise<McpState> =>
  invoke("unregister_mcp_project", { path }),
previewMcpChange: (request: McpChangeRequest): Promise<McpChangePreview> =>
  invoke("preview_mcp_change", { request }),
applyMcpChange: (request: McpChangeRequest): Promise<McpState> =>
  invoke("apply_mcp_change", { request }),
testMcpServer: (request: McpTestRequest): Promise<McpTestResult> =>
  invoke("test_mcp_server", { request }),
```

所有字段名与 Rust `#[serde(rename_all = "camelCase")]` 一一对应。

### 4.5 新增 `src/components/mcp/mcpForm.ts`

Before：文件不存在。

After：导出以下常量和纯函数：

```ts
import type {
  McpLocator,
  McpScope,
  McpService,
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

export function locatorKey(locator: McpLocator): string;
export function inferTransport(config: Record<string, unknown>): McpTransport;
export function redactConfig(
  config: Record<string, unknown>,
  sensitivePaths: string[]
): Record<string, unknown>;
export function restoreRedactedValues(
  candidate: Record<string, unknown>,
  previousRaw: Record<string, unknown>,
  sensitivePaths: string[]
): Record<string, unknown>;
export function validateLocatorForScope(
  locator: McpLocator
): Record<string, string>;
export function serviceSearchText(service: McpService): string;
```

实现规则写死：

1. `locatorKey` 按 `scope|instanceId|projectPath|name` 拼接，仅用于 React key，不作为后端身份令牌。
2. `inferTransport`：
   - `type === "http"` 或 `"streamable-http"` → `http`
   - `type === "sse"` → `sse`
   - `type === "ws"` → `ws`
   - `type === "stdio"`，或没有 `type` 且 `command` 是非空字符串 → `stdio`
   - 其他 → `unknown`
3. `redactConfig` 深拷贝，逐个 JSON Pointer 把值替换成 `REDACTED`；不得修改原对象。
4. `restoreRedactedValues` 只允许 `REDACTED` 出现在后端返回的 `sensitivePaths`；对应位置恢复 `previousRaw` 原值。新建服务中出现该保留字直接报错。
5. `validateLocatorForScope`：
   - User：禁止 `instanceId`、`projectPath`
   - Local：必须同时有 `instanceId`、`projectPath`
   - Project：必须有 `projectPath`，禁止 `instanceId`

### 4.6 新增 `src/components/mcp/McpPanel.tsx`

Before：文件不存在。

After：页面组件负责查询、筛选和动作编排，不负责配置文件语义。

#### 4.6.1 import

必须使用现有依赖提供的组件，不新增 UI 库：

```tsx
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Alert,
  Badge,
  Button,
  Card,
  Group,
  Loader,
  Menu,
  Modal,
  Select,
  SimpleGrid,
  Stack,
  Switch,
  Table,
  Text,
  TextInput,
  Title,
} from "@mantine/core";
import {
  IconAlertTriangle,
  IconCheck,
  IconDots,
  IconFolderPlus,
  IconPlus,
  IconRefresh,
  IconSearch,
  IconServerCog,
} from "@tabler/icons-react";
import { open } from "@tauri-apps/plugin-dialog";
import { notifications } from "@mantine/notifications";
import { api } from "../../api";
import type {
  McpChangeAction,
  McpChangePreview,
  McpChangeRequest,
  McpScope,
  McpService,
  McpState,
} from "../../api";
import McpImportModal from "./McpImportModal";
import McpServiceDrawer from "./McpServiceDrawer";
import {
  SCOPE_LABELS,
  TRANSPORT_LABELS,
  locatorKey,
  serviceSearchText,
} from "./mcpForm";
```

#### 4.6.2 页面状态

必须声明：

```tsx
const [state, setState] = useState<McpState | null>(null);
const [busy, setBusy] = useState(false);
const [err, setErr] = useState("");
const [query, setQuery] = useState("");
const [scope, setScope] = useState<McpScope | "all">("all");
const [instanceId, setInstanceId] = useState<string | "all">("all");
const [projectPath, setProjectPath] = useState<string | "all">("all");
type McpDrawerState =
  | { mode: "create" }
  | { mode: "edit" | "copy"; service: McpService };

const [drawerState, setDrawerState] = useState<McpDrawerState | null>(null);
const [importOpened, setImportOpened] = useState(false);
const [previewState, setPreviewState] = useState<{
  request: McpChangeRequest;
  preview: McpChangePreview;
} | null>(null);
const [applying, setApplying] = useState(false);
```

#### 4.6.3 查询和筛选

`load()` 必须：

1. `setBusy(true)`、清空 `err`。
2. 调用 `api.listMcpServices()`。
3. 成功写入 `state`；失败把 `String(e)` 写入 `err`。
4. `finally` 关闭 busy。
5. 首次挂载调用一次；刷新按钮复用同一函数。

列表筛选顺序：

1. scope。
2. instance；User/Project 没有实例时不因实例筛选被隐藏，只有 Local 按实例筛选。
3. project；User 不因项目筛选被隐藏，Local/Project 按路径筛选。
4. 名称、URL、command、项目路径、实例名的大小写不敏感搜索。
5. 排序：`disabled`、有 warning、`shadowed`、其余；同组按名称升序。

#### 4.6.4 页面结构

页面必须包含：

1. 标题「MCP 服务」和说明「管理用户级、项目本地和项目共享的 MCP 配置。」
2. 四个摘要卡：全部定义、已启用、存在警告、被覆盖。
3. 工具栏：
   - 搜索
   - 作用域筛选
   - 实例筛选
   - 项目筛选
   - 「添加项目」目录选择
   - 「导入 JSON」
   - 「添加 MCP」
   - 「刷新」
4. 表格列：
   - 服务名称
   - 作用域
   - 实例/项目
   - 类型
   - 生效状态
   - 启用开关
   - 操作
5. 每个 scope 使用固定 Badge：
   - User：蓝色「用户级」
   - Local：橙色「项目本地」
   - Project：青色「项目共享」
6. `shadowedBy` 非空时显示「被更高优先级覆盖」，悬浮信息列出覆盖来源。
7. User 的 `shadowedContextCount > 0` 显示「在 N 个上下文被覆盖」，不能把它标成全局失效。
8. `state.issues` 非空时在表格上方显示橙色 Alert；逐条显示文件路径和脱敏 detail。JSON 解析失败、User 副本未同步、目录不可访问都进入此处，禁止静默跳过。
9. 列标题使用「配置状态」，开关旁 tooltip 固定为「表示配置是否参与加载，不代表服务已连接或已通过项目审批」。

#### 4.6.5 添加项目

目录选择固定为：

```ts
const selected = await open({
  directory: true,
  multiple: false,
  title: "选择包含 MCP 配置的项目目录",
});
```

- 用户取消：不调用后端。
- 返回数组：视为异常，不取第一项。
- 返回字符串：调用 `api.registerMcpProject(selected)`，用返回的新 `McpState` 刷新页面。
- 后端校验目录存在、canonicalize 并持久化；前端不得自行拼接 `.mcp.json`。

#### 4.6.6 所有写操作统一流程

`prepareChange(action)` 必须：

1. 用当前 `state.revisions` 构造 `McpChangeRequest`。
2. 调 `previewMcpChange`。
3. 把 `{ request: { action, expectedRevisions: preview.expectedRevisions }, preview }` 写入 `previewState`。

页面内渲染 Mantine `Modal`，`opened={previewState !== null}`，固定展示：

1. `actionLabel`。
2. `affectedSources` 的 scope、路径。
3. `affectedInstances`。
4. JSON 格式化后的 `redactedBefore`、`redactedAfter`。
5. 全部 warnings。
6. 「取消」和「确认执行」按钮。

`confirmChange()` 必须：

1. 读取 `previewState.request`，设置 `applying=true`。
2. 调 `applyMcpChange`，不得重新使用页面初始 revisions。
3. 成功用返回值替换整个 `state`，清空 `previewState`，设置 `drawerState=null`，显示成功通知。
4. revision 冲突时显示「配置已被外部修改，请刷新后重试」，清空 `previewState`，调用 `load()`，不自动重放动作。
5. `finally` 设置 `applying=false`。

启停开关也必须走此流程，不允许直接乐观切换后静默失败。

### 4.7 新增 `src/components/mcp/McpServiceDrawer.tsx`

Before：文件不存在。

After：使用 Mantine `Drawer`，`position="right"`、`size="xl"`；新增、编辑、复制共用。

类型 import 固定为：

```ts
import type {
  McpChangeAction,
  McpService,
  McpState,
  McpTestResult,
} from "../../api";
```

Props：

```ts
interface McpServiceDrawerProps {
  opened: boolean;
  mode: "create" | "edit" | "copy";
  service?: McpService;
  state: McpState;
  onClose: () => void;
  onSave: (action: McpChangeAction) => Promise<void>;
  onTest: (name: string, config: Record<string, unknown>) => Promise<McpTestResult>;
}
```

#### 4.7.1 表单字段

| 区块 | 字段 | 规则 |
| --- | --- | --- |
| 基本信息 | 名称 | 新建必填；trim 后 1-64；仅字母、数字、`.`、`_`、`-`；拒绝保留名 |
| 基本信息 | 作用域 | User / Local / Project |
| 基本信息 | 实例 | Local 必填 |
| 基本信息 | 项目 | Local / Project 必填 |
| 连接方式 | transport | stdio / HTTP / SSE / WebSocket / 未知 |
| stdio | command | 非空 |
| stdio | args | 字符串数组，逐行编辑 |
| stdio | env | 键值表 |
| HTTP/SSE/WS | url | HTTP/SSE 只允许 http/https；WS 只允许 ws/wss |
| HTTP/SSE/WS | headers | 键值表 |
| 高级 | timeout | 可选整数；存在时必须 >= 1000 |
| 高级 | alwaysLoad | 可选布尔值 |
| 原始 JSON | 配置对象 | 顶层必须是对象；未知字段保留 |

保留名固定拒绝：

```ts
[
  "workspace",
  "claude-in-chrome",
  "computer-use",
  "Claude Preview",
  "Claude Browser",
]
```

#### 4.7.2 表单与原始 JSON 同步

1. 抽屉内部保留一份完整 `rawConfig`。
2. 表单只修改对应已知键，不使用新对象重建配置。
3. transport 从 stdio 切换到远程时，明确确认后删除 `command`、`args`、`env`；从远程切换到 stdio 时删除 `url`、`headers`。未知字段保持。
4. 原始 JSON 页签显示 `redactConfig(rawConfig, sensitivePaths)` 的结果。
5. 用户保存原始 JSON时，先调用 `restoreRedactedValues()` 恢复未修改凭据。
6. 用户需要替换凭据时，在 env/headers 键值表对应 PasswordInput 中输入新值；空值表示保留，显式「移除」按钮才删除。
7. 抽屉关闭或保存完成后，将 `rawConfig`、原始 JSON文本和测试结果全部重置。

#### 4.7.3 保存动作

- `mode=create`：`service` 为空，`original` 为空。
- `mode=edit`：`service` 必填；名称/作用域/实例/项目未变时 `original` 和 `target` 相同。
- `mode=edit` 且重命名或移动作用域：`original` 为旧 locator，`target` 为新 locator；后端在同一事务中删除旧定义并写入新定义。
- `mode=copy`：`service` 必填但 `original` 为空，target 名称预填 `${原名}-copy`，用户确认后保存。

### 4.8 新增 `src/components/mcp/McpImportModal.tsx`

Before：文件不存在。

After：Modal 只负责把用户粘贴内容解析成待保存定义，不直接写磁盘。

支持两种输入：

```json
{
  "type": "http",
  "url": "https://example.com/mcp"
}
```

```json
{
  "mcpServers": {
    "one": { "command": "node", "args": ["server.js"] },
    "two": { "type": "http", "url": "https://example.com/mcp" }
  }
}
```

规则：

1. 单个对象没有 `mcpServers` 时必须让用户填写服务名称。
2. 完整对象逐项生成导入预览。
3. 统一选择目标 scope；Local 还要选择实例，Local/Project 要选择项目。
4. 与目标 locator 同名时默认不勾选，并显示「已存在」；用户显式选择「覆盖」后才生成 save action。
5. 全部勾选项组成一个 `batchSave` action，只执行一次 `preview -> confirm -> apply`。
6. batch preview 按 source 分组展示所有新增/覆盖；apply 在同一直接文件事务中全部成功或全部回滚，User 项目只在直接写入完成后执行一次跨实例同步。
7. 输入文本、解析对象和完整凭据在 Modal 关闭后立即清空。
8. 错误信息只包含 JSON 路径和错误类型，不回显整个原始输入。
9. 粘贴文本 UTF-8 长度上限 1 MiB；完整对象最多 100 个服务，超过时拒绝并显示明确数量。

### 4.9 `src/glass.css`：新增 MCP 页面样式

定位：`src/glass.css:105-120,164-166,206-223`。

新增类名：

```css
.mcp-page { height: 100%; display: flex; flex-direction: column; gap: 16px; }
.mcp-summary-grid { flex: 0 0 auto; }
.mcp-toolbar { flex: 0 0 auto; display: flex; align-items: center; flex-wrap: wrap; gap: 10px; }
.mcp-table-card { flex: 1; min-height: 0; overflow: hidden; }
.mcp-table-scroll { height: 100%; overflow: auto; }
.mcp-name-cell { min-width: 180px; }
.mcp-source-cell { min-width: 220px; }
.mcp-config-summary { max-width: 360px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.mcp-scope-badges { display: flex; align-items: center; flex-wrap: wrap; gap: 6px; }
.mcp-form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
.mcp-kv-row { display: grid; grid-template-columns: minmax(120px, .8fr) minmax(180px, 1.4fr) auto; gap: 8px; align-items: end; }
.mcp-redacted { font-family: monospace; letter-spacing: 1px; }

@media (max-width: 900px) {
  .mcp-form-grid { grid-template-columns: 1fr; }
  .mcp-toolbar > * { flex: 1 1 180px; }
}
```

页面根节点使用：

```tsx
<div className="mcp-page">
```

表格区域自身滚动，不在页面外层再包 `view-scroll`，避免双滚动条。

### 4.10 新增 `src-tauri/src/mcp/mod.rs`

Before：文件不存在。

After：声明子模块、领域类型和六个 Tauri 命令。

```rust
mod storage;
mod validation;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpScope {
    User,
    Local,
    Project,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Http,
    Sse,
    Ws,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum McpEffectiveState {
    Effective,
    PartiallyShadowed,
    Shadowed,
    Disabled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpLocator {
    pub scope: McpScope,
    pub name: String,
    pub instance_id: Option<String>,
    pub project_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSaveItem {
    pub target: McpLocator,
    pub config: Map<String, Value>,
    pub overwrite: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum McpChangeAction {
    Save {
        original: Option<McpLocator>,
        target: McpLocator,
        config: Map<String, Value>,
        #[serde(default)]
        overwrite: bool,
    },
    BatchSave {
        items: Vec<McpSaveItem>,
    },
    SetEnabled {
        target: McpLocator,
        enabled: bool,
    },
    Delete {
        target: McpLocator,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpChangeRequest {
    pub action: McpChangeAction,
    pub expected_revisions: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpInstanceRef {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpProjectRef {
    pub path: String,
    pub label: String,
    pub discovered: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpShadowRef {
    pub scope: McpScope,
    pub name: String,
    pub instance_id: Option<String>,
    pub project_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpService {
    pub locator: McpLocator,
    pub transport: McpTransport,
    pub raw_transport: Option<String>,
    pub config: Map<String, Value>,
    pub enabled: bool,
    pub effective_state: McpEffectiveState,
    pub shadowed_by: Vec<McpShadowRef>,
    pub shadowed_context_count: usize,
    pub source_id: String,
    pub revision: String,
    pub sensitive_paths: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSummary {
    pub total: usize,
    pub enabled: usize,
    pub disabled: usize,
    pub warnings: usize,
    pub shadowed: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSourceIssue {
    pub source_id: String,
    pub path: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpState {
    pub services: Vec<McpService>,
    pub instances: Vec<McpInstanceRef>,
    pub projects: Vec<McpProjectRef>,
    pub revisions: BTreeMap<String, String>,
    pub issues: Vec<McpSourceIssue>,
    pub summary: McpSummary,
}
```

其余结构必须与 `src/api.ts` 同名字段对齐，禁止使用 `serde_json::Value` 直接代替整个响应：

- `McpInstanceRef`
- `McpProjectRef`
- `McpShadowRef`
- `McpService`
- `McpState`
- `McpSummary`
- `McpChangeAction`（`#[serde(tag = "op", rename_all = "camelCase")]`）
- `McpChangeRequest`
- `McpChangePreview`
- `McpTestRequest`
- `McpTestStage`
- `McpTestResult`

命令签名固定为：

```rust
#[tauri::command]
pub fn list_mcp_services() -> Result<McpState, String>;

#[tauri::command]
pub fn register_mcp_project(path: String) -> Result<McpState, String>;

#[tauri::command]
pub fn unregister_mcp_project(path: String) -> Result<McpState, String>;

#[tauri::command]
pub fn preview_mcp_change(
    request: McpChangeRequest,
) -> Result<McpChangePreview, String>;

#[tauri::command]
pub fn apply_mcp_change(
    request: McpChangeRequest,
) -> Result<McpState, String>;

#[tauri::command]
pub async fn test_mcp_server(
    request: McpTestRequest,
) -> Result<McpTestResult, String>;
```

命令只使用 `McpPaths::system()`；测试通过 `McpPaths::for_test(tempRoot)` 调内部函数，禁止测试读写真实 home。

#### 4.10.1 list 算法

按以下顺序执行：

1. 从现有 Profile 配置得到实例：主账户 `__main__` + `configured_profile_names()`。
2. 从 `~/.cc-manager/mcp-projects.json` 读取手工登记项目。
3. 扫描主账户和所有实例 `.claude.json` 的 `projects` 对象键，把存在的目录加入项目集合。
4. 读取主账户顶层 `mcpServers` 作为 User 权威视图；其他实例顶层只用于检测同步警告，不生成重复 User 行。
5. 对每个实例的每个 `projects[projectPath].mcpServers` 生成 Local 行。
6. 对每个已登记/发现项目读取 `<project>/.mcp.json.mcpServers` 生成 Project 行。
7. 合并 `mcp-disabled.json` 中的 User/Local 停用定义。
8. 读取每个项目 `.claude/settings.local.json.disabledMcpjsonServers` 标记 Project enabled。
9. 计算 transport、敏感路径、结构警告。
10. 按实例+项目上下文计算同名覆盖：
    - Local 行为最高优先级。
    - Project 行在同上下文有 Local 同名时 `shadowed`。
    - User 行统计被 Local/Project 同名覆盖的上下文数量；部分覆盖为 `partially-shadowed`，所有已知上下文均覆盖才为 `shadowed`。
11. 返回所有 source revision 和摘要。
12. 任一来源文件存在但无法读取/解析时，不覆盖该文件、不把它当空配置；写入 `McpSourceIssue`。User 各实例顶层 `mcpServers` 与主账户不一致时也写入 issue，提示下一次同步收敛。

摘要计数固定为：

- `total`：启用定义和停用仓库定义总数。
- `enabled` / `disabled`：按 `service.enabled` 计数。
- `warnings`：存在 `service.warnings` 的服务数加 `issues.len()`。
- `shadowed`：`effectiveState` 为 `shadowed` 或 `partially-shadowed` 的定义数。

Local 读取项目键时必须保留原始 JSON key：如果 `projects` 中已有路径与 canonical path 在当前平台等价，写回沿用已有 key；只有新建 Local 项目节点时才使用 canonical path。禁止因 Windows 大小写或路径分隔符差异生成第二个重复项目节点。

### 4.11 新增 `src-tauri/src/mcp/storage.rs`

Before：文件不存在。

#### 4.11.1 路径模型

```rust
pub(crate) struct McpPaths {
    pub home: PathBuf,
    pub manager_dir: PathBuf,
    pub profiles_path: PathBuf,
}
```

提供：

```rust
impl McpPaths {
    pub fn system() -> Self;
    #[cfg(test)]
    pub fn for_test(root: PathBuf) -> Self;

    pub fn main_claude_json(&self) -> PathBuf;
    pub fn instance_claude_json(&self, instance_id: &str) -> Result<PathBuf, String>;
    pub fn project_mcp_json(&self, project_path: &str) -> Result<PathBuf, String>;
    pub fn project_local_settings(&self, project_path: &str) -> Result<PathBuf, String>;
    pub fn disabled_store(&self) -> PathBuf;
    pub fn project_registry(&self) -> PathBuf;
    pub fn backup_dir(&self) -> PathBuf;
}
```

路径安全规则：

1. instance 必须是 `__main__` 或现有 Profile 名称。
2. project path 必须 canonicalize、必须是目录，并且存在于项目注册表或已发现的 `.claude.json.projects` 中。
3. 后端忽略前端传入的 sourceId 文件路径；sourceId 由后端根据 locator 重新计算。
4. 不允许 `..`、不存在路径或文件路径注册为项目。
5. `unregister` 只移除项目注册记录，不删除项目 `.mcp.json`、`.claude` 或任何业务文件。

#### 4.11.2 项目注册表

`~/.cc-manager/mcp-projects.json` 固定格式：

```json
{
  "version": 1,
  "projects": [
    "E:\\work\\project-a"
  ]
}
```

保存前排序、去重；路径比较在 Windows 忽略大小写，在 macOS 保留大小写。

#### 4.11.3 停用仓库

`~/.cc-manager/mcp-disabled.json` 固定格式：

```json
{
  "version": 1,
  "entries": [
    {
      "scope": "user",
      "name": "github",
      "instanceId": null,
      "projectPath": null,
      "config": {},
      "disabledAt": 1784851200
    }
  ]
}
```

约束：

- 只存 User 和 Local；Project 使用官方 `disabledMcpjsonServers`。
- key 为 `scope + instanceId + projectPath + name`。
- 停用先把完整配置写入仓库并成功落盘，再从启用来源移除。
- 启用先写回目标来源并成功落盘，再从仓库删除。
- 任一步失败时回滚到操作前的两个文档。

#### 4.11.4 revision

revision 固定为字符串：

```text
<modified-unix-nanos>:<file-length>:<content-hash-u64>
```

文件不存在固定为：

```text
missing
```

`content-hash-u64` 使用 `std::collections::hash_map::DefaultHasher` 对完整原始 bytes 计算，只用于并发变更检测，不作为安全签名。`preview` 和 `apply` 都重新读取 revision。`apply` 中任一受影响 source 与 `expectedRevisions` 不一致即整体拒绝，不写任何文件。

sourceId 固定格式如下，作为不透明字符串返回前端，后端不得通过解析前端 sourceId 决定真实路径：

```text
user:__main__
user:<instanceId>
local:<instanceId>:<canonicalProjectPath>
project:<canonicalProjectPath>
project-settings:<canonicalProjectPath>
manager:disabled
manager:projects
```

受影响 revision 集合固定为：

- User save/enable/disable/delete：主账户、全部现有实例的 User source，加 `manager:disabled`（仅启停涉及）。
- Local save/delete：目标 Local source；Local enable/disable再加 `manager:disabled`。
- Project save/delete：目标 Project source；Project enable/disable再加 Project settings source。
- 移动 scope 或重命名：original 与 target 两侧集合的并集。
- BatchSave：全部 item target 集合的并集。
- 项目登记/取消登记：`manager:projects`。

#### 4.11.5 备份和原子写入

新增：

```rust
pub(crate) fn write_json_transactional(
    paths: &McpPaths,
    target: &Path,
    value: &Value,
) -> Result<(), String>;
```

固定步骤：

1. 序列化 pretty JSON。
2. 在 target 同目录写入唯一临时文件并 `sync_all()`。
3. target 存在时复制到 `~/.cc-manager/mcp-backups/<source-hash>/<timestamp>.json`。
4. 每个 source-hash 只保留最近 5 份，按文件名时间排序删除更旧备份。
5. Windows/macOS 都采用「原文件 rename 到 rollback → 临时文件 rename 为 target → 成功后删除 rollback」。
6. 第二次 rename 失败时把 rollback 恢复为 target。
7. 不使用 `remove_file(target)` 后再 rename，避免无保护窗口。
8. 项目 `.mcp.json` 的备份不写在项目目录，避免污染 Git 工作区。

备份可能包含原配置凭据，继承 `~/.cc-manager` 的用户权限；诊断导出不得包含备份内容。

多文件动作由 `apply_storage_transaction()` 编排：

1. 在写入前读取全部直接受影响文件的原始 bytes 和 revision。
2. 按稳定 sourceId 顺序逐文件调用 `write_json_transactional()`。
3. 任一直接写入失败时，按相反顺序恢复已写文件；原先不存在的文件恢复为不存在。
4. User 跨实例同步不承诺多副本原子提交：主账户写入成功后作为目标状态，部分实例写回失败时保留主账户结果并返回同步 warning，下一次 `syncAll/--sync` 继续收敛。
5. preview 文案必须区分「直接修改文件」和「随后同步实例」，禁止宣称所有实例在一个文件事务中完成。

#### 4.11.6 作用域读写

User：

```text
~/.claude.json
└─ mcpServers[name]
```

Local：

```text
<instance .claude.json>
└─ projects[canonicalProjectPath].mcpServers[name]
```

Project：

```text
<project>/.mcp.json
└─ mcpServers[name]
```

写入只修改目标节点，保留文档其他字段。删除服务后：

- `mcpServers` 变空时保留空对象，避免重建父级时丢失语义。
- Local 的项目对象还有其他字段时必须保留。
- `.claude.json` 顶层其他登录态、权限、缓存字段不得变化。
- Project 停用时若 `<project>/.claude` 不存在，创建目录后再写 `settings.local.json`。
- 不自动修改项目 `.gitignore`；preview 固定提示「.claude/settings.local.json 是本机停用记录，请保持未提交」，真机验证必须检查 Git 状态。

### 4.12 新增 `src-tauri/src/mcp/validation.rs`

Before：文件不存在。

After 导出：

```rust
pub(crate) fn validate_name(name: &str) -> Result<String, String>;
pub(crate) fn infer_transport(config: &Map<String, Value>) -> McpTransport;
pub(crate) fn validate_config(config: &Map<String, Value>) -> Vec<McpTestStage>;
pub(crate) fn sensitive_paths(config: &Value) -> Vec<String>;
pub(crate) fn redact(config: &Value, paths: &[String]) -> Value;
pub(crate) fn test_basic(
    name: &str,
    config: &Map<String, Value>,
) -> McpTestResult;
```

#### 4.12.1 名称校验

- trim 后 1-64 字符。
- 正则语义：首字符和后续字符均只允许 ASCII 字母、数字、`.`、`_`、`-`。
- 拒绝 `workspace`、`claude-in-chrome`、`computer-use`、`Claude Preview`、`Claude Browser`。
- 读取既有配置时不因旧名称非法而隐藏；显示 warning，但重命名以外的保存允许保留旧名称。

#### 4.12.2 配置校验

stdio：

- `command` 必须非空字符串。
- `args` 存在时必须为字符串数组。
- `env` 存在时必须为字符串值对象。

HTTP/SSE：

- `url` 必须是 http/https。
- `headers` 存在时必须为字符串值对象。
- SSE 追加 warning「SSE 已弃用，服务支持时请迁移到 HTTP」。

WebSocket：

- `url` 必须是 ws/wss。
- `headers` 存在时必须为字符串值对象。

通用：

- `timeout` 存在时必须是 >= 1000 的整数。
- `alwaysLoad` 存在时必须是布尔值。
- 未知字段不报错、不删除。
- URL 存在但 type 缺失时必须判错，不能误认为 stdio。
- 单个 config pretty JSON 序列化后不得超过 256 KiB。

#### 4.12.3 敏感字段

以下 key 大小写不敏感匹配为敏感：

```text
authorization
proxy-authorization
cookie
set-cookie
token
access_token
refresh_token
secret
client_secret
password
passwd
api_key
apikey
credential
```

同时匹配：

- `headers.Authorization` 和 `headers.Proxy-Authorization` 的所有值。
- env key 包含 `TOKEN`、`SECRET`、`PASSWORD`、`API_KEY`、`APIKEY`、`CREDENTIAL`。

返回 RFC 6901 JSON Pointer；错误、preview、测试 detail 和 `sync.log` 写入前都通过同一 redaction 函数。

#### 4.12.4 基础测试

stdio：

1. 完成 schema 校验。
2. command 是绝对路径：检查存在且是文件。
3. command 非绝对路径：按当前 GUI 进程 PATH 搜索；Windows 同时检查 `.exe`、`.cmd`、`.bat`。
4. Windows 发现 `command === "npx"` 时返回 warning：「Windows 原生环境请把 command 改为 cmd，并把 /c、npx 放在 args 最前面」；不自动重写。
5. 不启动进程。

HTTP/SSE/WS：

1. 完成 schema 校验。
2. 用 `url` crate 解析 host 和 port。
3. HTTP/WS 默认 80，HTTPS/WSS 默认 443。
4. 在 `spawn_blocking` 内执行 DNS 解析和 `TcpStream::connect_timeout`，超时 3 秒。
5. 不发送 HTTP Header、Token、Cookie 或 MCP 请求。
6. TCP 可连接只表示基础网络可达，结果文案固定为「端点可达，尚未执行 MCP 握手」。

配置包含 `${VAR}` 或 `${VAR:-default}` 时：

- schema 校验继续执行。
- command/url 的存在性或网络测试标记为 `skipped`。
- detail 固定为「包含运行时环境变量，未展开，跳过基础可达性检查」。
- 不读取或展开用户环境变量，避免测试结果与 Claude Code 实际启动环境产生伪一致。

因此 `src-tauri/Cargo.toml` 额外增加：

```toml
url = "2"
```

不引入 `reqwest`、MCP SDK 或异步 HTTP 客户端。

### 4.13 `src-tauri/src/sync.rs`：让 User 写入与既有同步共用锁

定位：`src-tauri/src/sync.rs:396-470`。

现状问题：`sync_configs()` 内部自行拿锁，新 MCP 命令若先写主配置再调用它，中间存在 CLI `--sync` 插入的竞态窗口。

Before：

```rust
struct LockGuard(PathBuf);

fn acquire_lock() -> Option<LockGuard> {
    let p = lock_path();
    let _ = fs::create_dir_all(crate::cfg_dir());
    for _ in 0..2 {
        match fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(mut f) => {
                let _ = write!(f, "{}", std::process::id());
                return Some(LockGuard(p.clone()));
            }
            Err(_) => {
                let stale = fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .map(|d| d.as_secs() > 60)
                    .unwrap_or(true);
                if stale {
                    let _ = fs::remove_file(&p);
                    continue;
                }
                return None;
            }
        }
    }
    None
}

pub fn sync_configs(names: &[String]) -> Result<String, String> {
    let _guard = match acquire_lock() {
        Some(g) => g,
        None => return Ok("另一个同步正在进行,本轮跳过".into()),
    };
    let snap: Snapshot = fs::read_to_string(snapshot_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    // 后续继续构造 mcp_files/plugin_files、调用 sync_domain 并写 snapshot。
}
```

After 结构：

```rust
pub(crate) struct ConfigLockGuard(PathBuf);

pub(crate) fn acquire_config_lock() -> Option<ConfigLockGuard> {
    // 把现有 acquire_lock 函数体完整迁移到这里，不改变 60 秒陈旧锁规则
}

pub(crate) fn sync_configs_locked(names: &[String]) -> Result<String, String> {
    // 把现有 sync_configs 中读取快照至返回 summary 的代码完整迁移到这里
}

pub fn sync_configs(names: &[String]) -> Result<String, String> {
    let _guard = match acquire_config_lock() {
        Some(guard) => guard,
        None => return Ok("另一个同步正在进行,本轮跳过".into()),
    };
    sync_configs_locked(names)
}
```

`mcp::apply_mcp_change` 处理 User save/enable/disable/delete 时：

1. 严格获取 `acquire_config_lock()`；拿不到直接报错，不返回“跳过”成功。
2. 在锁内校验 revision。
3. 写主账户 User 配置/停用仓库。
4. 调 `sync_configs_locked(configured_profile_names())`。
5. 重新读取状态后返回。

Local 写入也获取同一锁，避免与 `sync_configs_locked` 同时重写同一个实例 `.claude.json`。Project `.mcp.json` 不参与跨实例同步，但仍走 MCP 自身 revision 和事务写入。

### 4.14 `src-tauri/src/main.rs`：注册模块、插件和命令

定位：`src-tauri/src/main.rs:8-10,913-948,1464-1501`。

#### 4.14.1 模块

Before：

```rust
mod claude_cli;
mod health;
mod sync;
```

After：

```rust
mod claude_cli;
mod health;
mod mcp;
mod sync;
```

为让 `mcp` 复用现有路径，以下函数改为 `pub(crate)`，函数体不变：

```rust
pub(crate) fn home() -> PathBuf;
pub(crate) fn cfg_dir() -> PathBuf;
```

`Profile.name` 通过受控函数读取，新增：

```rust
pub(crate) fn configured_profile_names() -> Vec<String> {
    profile_names(&load())
}
```

`load()`、`profile_names()` 和 `Profile` 保持私有；`mcp` 只调用 `configured_profile_names()`，不接触 `token_enc`。

#### 4.14.2 extension_overview

删除 `main.rs:931-947` 的 MCP 读取和 MCP `groups.push` 调用，函数只返回 Skills、Plugins、Agents、Commands。

#### 4.14.3 插件注册

Before：

```rust
.plugin(tauri_plugin_updater::Builder::new().build())
.plugin(tauri_plugin_process::init())
```

After：

```rust
.plugin(tauri_plugin_updater::Builder::new().build())
.plugin(tauri_plugin_process::init())
.plugin(tauri_plugin_dialog::init())
```

#### 4.14.4 invoke handler

在 `extension_overview` 后注册：

```rust
mcp::list_mcp_services,
mcp::register_mcp_project,
mcp::unregister_mcp_project,
mcp::preview_mcp_change,
mcp::apply_mcp_change,
mcp::test_mcp_server,
```

### 4.15 后端动作事务规则

#### Save

1. 校验 target locator 和 config。
2. 如果有 original，解析 original 和 target 的全部受影响 source。
3. 校验所有 revision。
4. 检查 target 是否存在：
   - original 与 target 相同：更新。
   - original 不同且 target 已存在：`overwrite !== true` 时拒绝；导入流程中用户明确勾选覆盖后传 `overwrite: true`。
5. 同一事务内删除 original、写 target。
6. User 涉及变更时同步全部实例。

#### BatchSave

1. `items` 必须为 1-100 项；空数组或超过 100 项拒绝。
2. 所有 target locator 必须互不重复。
3. 逐项执行与 Save 相同的 locator、名称、config 和 overwrite 校验。
4. 受影响 source 为全部 target source 的并集。
5. 直接文件按 sourceId 分组，每个文件只解析和写入一次。
6. 任一项失败时不写任何直接文件；写入阶段失败按 `apply_storage_transaction()` 全部回滚。
7. 包含 User 项时，直接文件事务成功后只执行一次 `sync_configs_locked()`。

#### SetEnabled

- User/Local：按 4.11.3 在启用来源和停用仓库之间移动。
- Project：
  - disabled：向 `<project>/.claude/settings.local.json.disabledMcpjsonServers` 追加 name、排序、去重。
  - enabled：移除 name；数组变空时删除 `disabledMcpjsonServers` 键，其他 settings 完整保留。

#### Delete

- 删除启用定义或停用仓库定义。
- Project 删除 `.mcp.json` 条目时，同时从 `disabledMcpjsonServers` 移除同名残留。
- User 删除后执行跨实例同步。
- 删除确认 preview 必须显示实际文件和影响范围。
- 不提供批量删除。

## 5. 关键状态 / 标记的完整生命周期

| 状态/文件 | 何时产生 | 何时消费 | 何时清理 | 进程被杀时行为 |
| --- | --- | --- | --- | --- |
| `McpState` | 进入页面或刷新 | 列表、筛选、编辑、revision | 页面卸载 | 仅内存，重进重读 |
| `drawerService/rawConfig` | 打开新建/编辑/复制抽屉 | 表单和原始 JSON | 关闭或保存成功 | 仅前端内存，不落盘 |
| `sensitivePaths` | 后端读取配置时计算 | UI 脱敏、preview、错误清理 | 随 `McpState` 清理 | 可重新计算，不泄漏 |
| `expectedRevisions` | list/preview 返回 | apply 前并发校验 | apply 成功后由新 state 替换 | 过期后 apply 被拒绝 |
| `mcp-projects.json` | 用户添加项目 | scope 选择、Project 扫描 | 用户取消登记 | 进程被杀后仍保留；不含凭据 |
| `mcp-disabled.json` | User/Local 停用 | 列表、重新启用、删除 | 启用或彻底删除 | 事务写入，保留可恢复配置 |
| `disabledMcpjsonServers` | Project 停用 | Claude Code 和列表状态 | Project 启用/删除 | 官方配置持久化 |
| `ConfigLockGuard` | User/Local apply 或全局同步 | 串行化 `.claude.json` 写入 | RAII Drop | 正常退出自动删；崩溃后 60 秒陈旧锁自愈 |
| 临时 JSON 文件 | 事务写入开始 | rename 提升 | 成功或错误回滚后 | 下次写入先清理同 source 遗留临时文件 |
| rollback 文件 | 替换既有文件前 | 写失败恢复 | 成功后删除 | 下次读取发现 target 缺失且 rollback 有效时恢复 |
| MCP 备份 | 每次覆盖已有文件前 | 手工恢复/故障排查 | 每 source 保留最近 5 份 | 持久保留；不进入诊断导出 |
| 基础测试结果 | 用户点击测试 | 抽屉展示 | 关闭抽屉、修改关键配置后清空 | 不落盘 |
| 导入原文 | 打开导入并粘贴 | 解析预览 | Modal 关闭/完成 | 仅前端内存 |

### 5.1 凭据泄漏边界论证

1. 配置文件和备份仍可能包含明文凭据，这是用户选择的 4A 兼容模式，属于现状存储语义，不新增加密承诺。
2. UI 只渲染脱敏值；PasswordInput 不显示已有明文。
3. 前端抽屉内存保留完整 rawConfig 是可接受边界：为了无损编辑必须存在，关闭抽屉立即释放，不写 localStorage/sessionStorage。
4. Rust 错误不得使用 `format!("{config:?}")`、不得返回原始 JSON。
5. preview、通知、同步日志、诊断导出只能接收 redacted Value。
6. 基础网络测试不发送 headers/env，因此测试过程不会把 Token 发到外部。
7. React key、sourceId、revision 不包含 config 值。

## 6. 明确不改动项

1. `src-tauri/src/main.rs:689-749` 的 `store_token`、`clear_token` 不改。
2. `src-tauri/src/main.rs:1119-1169` 的 `decrypt_token` 和模型检测不改。
3. `Profile.token_enc` 结构、网关 Token 保存方式、PowerShell SecureString 和 macOS Keychain 路径不改。
4. `generate_sh`、`generate_ps1` 的 Claude 实例启动脚本不改。
5. `ConfigPanel` 的实例管理、网关、模型和权限设置不改。
6. Usage、Diagnostics、Settings、Guide 页面不改。
7. 现有 `enabledPlugins` 三方同步语义不改。
8. Local/Project MCP 不加入顶层 `mcpServers` 三方合并。
9. 不修改 CSP；远程基础测试在 Rust 侧执行。
10. 不向项目目录写备份文件。

## 7. 已知范围外

1. 不实现完整 MCP 握手、工具/资源/提示词发现。
2. 不启动 stdio MCP 子进程。
3. 不发送 HTTP/SSE/WebSocket MCP 请求。
4. 不处理 OAuth 登录、Token 刷新或浏览器回调。
5. 不管理企业 `managed-mcp.json`。
6. 不枚举或编辑插件自带 MCP。
7. 不管理 claude.ai connectors。
8. 不安装、下载、推荐或展示第三方 MCP 市场。
9. 不递归扫描磁盘寻找所有 `.mcp.json`；只处理已登记项目和 `.claude.json.projects` 已知项目。
10. 不执行 Git add/commit；Project 修改只写文件并提示用户自行检查 Git diff。
11. 不实现常驻健康监控；测试结果不持久化。
12. 不把凭据迁移到系统凭据库。
13. 不汇总企业 managed settings 或其他外部 settings 对 MCP 的 allow/deny 策略；Project 行只反映本功能管理的项目 `.claude/settings.local.json` 停用状态。实际审批与策略结果以 Claude Code `/mcp` 为准。

## 8. 验证清单

### 8.1 前端主场景

- [ ] 侧边栏出现独立「MCP 服务」，位于空间和扩展之间。
- [ ] 扩展中心不再展示 MCP 卡片或 MCP 数量。
- [ ] 页面显示 User、Local、Project 三种明确 Badge。
- [ ] Local 行同时显示实例和项目。
- [ ] Project 行显示项目路径和「修改项目文件」提示。
- [ ] 同名 User/Project/Local 分三行展示。
- [ ] 相同上下文中覆盖关系严格为 Local > Project > User。
- [ ] 搜索、scope、实例、项目筛选可以组合。
- [ ] 新建、编辑、复制、重命名、移动 scope 均走 preview。
- [ ] revision 冲突后不覆盖，页面自动刷新。
- [ ] 凭据在列表、抽屉、原始 JSON、preview、错误通知中均不显示明文。
- [ ] 导入完整 `mcpServers` 能逐项预览和保存。
- [ ] 未知字段保存前后字节语义不丢失。

### 8.2 后端主场景

- [ ] User 写入主账户顶层后同步到全部实例。
- [ ] User 删除不会被其他实例旧副本重新“复活”。
- [ ] Local 只修改指定实例、指定项目。
- [ ] Project 只修改 `<project>/.mcp.json`。
- [ ] Project 停用不删除 `.mcp.json` 条目，只修改 `.claude/settings.local.json`。
- [ ] User/Local 停用后配置可完整恢复。
- [ ] 非法 instance/project path 被拒绝。
- [ ] 目标文件的其他字段完整保留。
- [ ] Windows 覆盖既有 JSON 文件成功，失败时 rollback 可恢复。
- [ ] 每个源最多保留 5 份备份。
- [ ] 基础测试不发送凭据。

### 8.3 回归断言

- [ ] 应用启动 `syncAll()` 行为保持。
- [ ] `--sync` CLI 模式保持无 GUI、无 panic。
- [ ] Skills/Plugins/Agents/Commands 共享目录保持。
- [ ] enabledPlugins 合并测试全部通过。
- [ ] Profile 新建、保存、删除不受影响。
- [ ] Profile Token 加密、解密、模型检测不受影响。
- [ ] 实例 settings revision 冲突保护保持。
- [ ] Usage、更新、诊断、证书功能可正常打开。

### 8.4 必须真机验证

- [ ] Windows：目录选择器返回正常绝对路径。
- [ ] macOS：目录选择器返回正常绝对路径。
- [ ] Windows：`cmd /c npx` 能被 basic test 识别；裸 `npx` 显示 warning。
- [ ] Windows：覆盖已存在 `.claude.json`、`.mcp.json` 不触发 rename 失败。
- [ ] macOS：`~/.cc-manager`、项目 `.mcp.json` 权限符合当前用户。
- [ ] 两个 Claude 实例同时存在时，User 增删改同步收敛。
- [ ] Claude CLI 正在执行 `--sync` 时，GUI 写入明确失败或等待下一次操作，不静默丢失。
- [ ] 项目 `.mcp.json` 修改后，除首次 pretty JSON 格式化外，语义变化只涉及目标 `mcpServers`；其他顶层字段值保持相等。

### 8.5 单元测试清单

在 `mcp/storage.rs`、`mcp/validation.rs` 和 `sync.rs` 增加测试：

1. 三种 scope 的路径解析。
2. 非注册项目、文件路径、`..` 拒绝。
3. Local 不影响其他实例。
4. Project 只改 `.mcp.json`。
5. User/Local 停用往返无损。
6. Project 停用只改 `disabledMcpjsonServers`。
7. revision 变化拒绝 apply，包含同长度、同时间粒度下内容变化的回归测试。
8. unknown 字段保留。
9. sensitive JSON Pointer 和 redaction。
10. URL 缺 type 判错。
11. stdio command/args/env 校验。
12. HTTP/SSE/WS scheme 校验。
13. reserved name 拒绝。
14. 同名优先级和 `partially-shadowed`。
15. User 删除在快照同步后不复活。
16. 事务写入失败时 rollback 恢复。
17. 备份轮换只保留 5 份。

## 9. 影响文件清单

| 文件 | 改动 |
| --- | --- |
| `package.json` | 增加 Dialog 前端依赖 |
| `pnpm-lock.yaml` | 安装命令自动更新 |
| `src-tauri/Cargo.toml` | 增加 Dialog 和 `url` 依赖 |
| `src-tauri/Cargo.lock` | 安装/测试命令自动更新 |
| `src-tauri/capabilities/default.json` | 开放目录选择 |
| `src/App.tsx` | 新增一级菜单、标题和页面渲染 |
| `src/api.ts` | 新增 MCP 类型与 IPC |
| `src/components/ExtensionsPanel.tsx` | 移除 MCP 概览 |
| `src/components/mcp/McpPanel.tsx` | 新增 MCP 管理页 |
| `src/components/mcp/McpServiceDrawer.tsx` | 新增编辑抽屉 |
| `src/components/mcp/McpImportModal.tsx` | 新增 JSON 导入 |
| `src/components/mcp/mcpForm.ts` | 新增表单/脱敏工具 |
| `src/glass.css` | 新增 MCP 页面样式 |
| `src-tauri/src/main.rs` | 注册模块、插件、命令；概览移除 MCP |
| `src-tauri/src/sync.rs` | 抽出可复用配置锁和 locked 同步入口 |
| `src-tauri/src/mcp/mod.rs` | 新增领域命令与编排 |
| `src-tauri/src/mcp/storage.rs` | 新增多作用域存储 |
| `src-tauri/src/mcp/validation.rs` | 新增校验、脱敏、基础测试 |

校验命令固定为：

```powershell
pnpm typecheck
```

```powershell
Set-Location src-tauri
cargo test
```

不使用 `pnpm build` 作为本方案的验证命令。真机验证完成前不得把“typecheck + cargo test 通过”描述为功能已经可用。

## 10. 落地顺序

必须按以下顺序实施，禁止先做页面再临时补后端语义：

1. 安装 Dialog 依赖并完成权限注册。
2. 新增 Rust `mcp` 类型、路径模型、revision 和测试临时路径。
3. 完成三种 scope 读取与覆盖关系单元测试。
4. 完成备份、事务写入和停用仓库。
5. 抽取 sync 配置锁，完成 User 写入同步测试。
6. 完成 preview/apply/test Tauri 命令。
7. 在 `src/api.ts` 对齐类型和 IPC。
8. 完成 `mcpForm.ts`。
9. 完成 `McpServiceDrawer` 和 `McpImportModal`。
10. 完成 `McpPanel`。
11. 接入 App 一级菜单并移除扩展 MCP 卡片。
12. 补充样式，执行 typecheck、cargo test。
13. 在 Windows 和 macOS 执行真机清单。

## 11. 文档审核记录

审核日期：2026-07-24。

本次审核只验证方案文档的完整性和可执行性，不代表业务代码已经落地或通过验收。审核结果：

- [x] 已覆盖负责人确认的 `1B / 2A / 3A / 4A`。
- [x] 已把 User、Local、Project 的存储位置、身份字段和停用语义分别写死。
- [x] 已写明 Local > Project > User 的覆盖关系，未把同名定义错误合并。
- [x] 已把 User 写入与既有三方同步锁、快照删除传播连接起来。
- [x] 已补齐 revision、preview、直接文件事务、跨实例最终收敛的边界。
- [x] 已补齐凭据在 UI、preview、错误、通知和日志中的脱敏边界。
- [x] 已补齐项目目录登记、canonical path 和 Local 原始项目 key 保留规则。
- [x] 已补齐 JSON 导入的 BatchSave 原子语义和数量/体积上限。
- [x] 已补齐 Windows rename、rollback、备份轮换和真机验证。
- [x] 已核对前端 IPC 名称与 Rust Tauri 命令一一对应。
- [x] 必含章节、代码围栏和影响文件清单检查通过。
- [x] 未发现 TODO、待定项或要求落地模型自行选择的设计分叉。

文档审核结论：方案内容可交给其他 AI 按顺序实施；元信息继续保持「草稿·待落地」，直到业务代码完成并按本文档执行实现审核。
