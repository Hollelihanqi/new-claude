import type { FeatureHelpContent } from "./FeatureHelp";

export const SKILLS_HELP: FeatureHelpContent = {
  title: "共享 Skills",
  hint: "了解 Skill 如何提供给 Claude 环境和 Codex",
  purpose: "把一项可重复使用的工作方法提供给所有受管理的 Claude 环境，并同时提供给 Codex。",
  action: "新增或更新共享 Skill 后，应用会立即更新所有仍在使用共享版本的目标。",
  impact: "影响所有未排除、也没有同名本地版本的 Claude 环境，以及 Codex 的个人 Skill。默认 Claude 只读，不会被修改。",
  userAction: "一般不需要操作。已经有同名本地 Skill 的环境会继续使用自己的版本；需要改回共享版本时再点“恢复继承”。",
  flow: [
    "应用读取共享库中的一个完整 Skill。",
    "逐个检查 Claude 环境与 Codex 是否已有同名本地版本。",
    "没有覆盖的目标收到新版本；有本地版本的目标保持原样。",
    "应用记录本次分发结果，下一次更新据此判断文件是否被用户修改。",
  ],
  principles: [
    {
      title: "完整 Skill 是最小单位",
      detail: "一个 Skill 目录整体更新，不把两个同名 Skill 的文件拼在一起，避免得到无法解释的混合版本。",
      status: "implemented",
    },
    {
      title: "环境自己的版本优先",
      detail: "应用会记住上次分发的内容。环境文件与记录不同时，就认为用户改过，后续共享更新不会覆盖它。",
      status: "implemented",
    },
    {
      title: "运行中发现变化",
      detail: "应用会更新目标中的真实目录项，让 Claude Code 的文件监听能够看到变化；Windows 与 macOS 的运行中会话仍要分别验证。",
      status: "verification",
    },
  ],
  example: "例如共享 reviewer Skill 更新后，corp 和 test 环境会马上收到新版本；如果 test 自己改过 reviewer，它继续保留自己的版本。",
  failure: "如果复制、校验或切换失败，旧版本保持不变，并明确列出失败的目标。",
  codeEntries: ["src-tauri/src/extensions.rs", "src/components/ExtensionsPanel.tsx"],
};

export const PLUGINS_HELP: FeatureHelpContent = {
  title: "插件管理",
  hint: "了解插件为什么要在每个环境分别安装",
  purpose: "查看默认 Claude 和各环境的真实插件状态，并把指定插件安装、启用或停用到选择的环境。",
  action: "应用为每个目标环境设置它自己的 Claude 配置目录，再调用 Claude Code 官方插件命令。",
  impact: "每个环境拥有独立插件文件和安装记录。一个环境安装失败不会伪装成全部成功，也不会修改默认 Claude。",
  userAction: "安装、启停或卸载后，已打开的会话执行 /reload-plugins；更新插件后按 Claude Code 官方要求重启会话。包含 monitors 的插件也需要重启会话。",
  flow: [
    "只读列出默认 Claude 或用户输入的插件名称。",
    "为目标环境设置独立的 CLAUDE_CONFIG_DIR。",
    "调用 Claude Code 官方 plugin 命令逐环境处理。",
    "重新读取每个环境的真实状态，并逐项展示成功或失败。",
  ],
  principles: [
    {
      title: "不复制插件目录",
      detail: "插件的安装记录可能包含环境路径和缓存位置。复制目录容易出现“列表里有、实际不能用”，因此安装必须交给 Claude Code 完成。",
      status: "implemented",
    },
    {
      title: "逐环境报告",
      detail: "“所有环境”会循环执行同一个操作。每个环境独立成功或失败，不为了表面一致而回滚已经正确安装的环境。",
      status: "implemented",
    },
    {
      title: "不带网关凭据",
      detail: "插件管理命令只收到目标配置目录，不携带网关地址、API Key 或网关证书。",
      status: "implemented",
    },
  ],
  example: "例如默认 Claude 装有 formatter@marketplace，选择 corp 和 test 后，Claude Code 会在两个环境中各安装一次。",
  failure: "如果 corp 成功而 test 失败，界面会分别显示结果，并保留 corp 的成功安装。",
  codeEntries: ["src-tauri/src/extensions.rs", "src-tauri/src/claude_cli.rs", "src/components/ExtensionsPanel.tsx"],
};

export const AGENTS_HELP: FeatureHelpContent = {
  title: "共享 Agents",
  hint: "了解 Agent 与 Skill、MCP 的关系",
  purpose: "提供有明确职责的专业助手，例如代码审核员或测试员，让 Claude 把合适的任务交给它。",
  action: "共享 Agent 会分发给所有 Claude 环境。它在独立上下文中工作，完成后把结果交回当前对话。",
  impact: "只影响 Claude Code 环境，不会被转换成 Codex 任务。环境可以覆盖、排除，或保留环境独有 Agent。",
  userAction: "如果显示缺少 Skill 或 MCP，需要按提示补齐；应用不会擅自安装或打开依赖。",
  flow: [
    "应用校验 Agent 文件中的名称、模型、工具限制和依赖声明。",
    "完整文件分别写入仍使用共享版本的 Claude 环境。",
    "Claude 在需要时创建独立上下文运行 Agent。",
    "Agent 只把最终结果交回原对话，不把整个思考过程塞进主上下文。",
  ],
  principles: [
    {
      title: "复制并记录来源",
      detail: "Windows 对文件链接有额外限制，因此使用完整文件复制，并记录应用上次写入的内容，用来识别环境是否自行修改。",
      status: "implemented",
    },
    {
      title: "依赖只检查、不代办",
      detail: "Agent 可以声明需要某个 Skill 或 MCP。缺少时显示不可用和具体原因，不自动改变环境的工具权限。",
      status: "implemented",
    },
    {
      title: "Claude 与 Codex 不混用",
      detail: "Claude 自定义 Agent 文件和 Codex 的任务机制不是同一种配置，强行转换会改变含义，因此只分发到 Claude 环境。",
      status: "implemented",
    },
  ],
  example: "例如 code-reviewer Agent 依赖 git MCP。corp 已配置该 MCP 时可以使用；test 缺少时会显示缺少依赖。",
  failure: "某个环境写入失败时只报告该环境；其他环境的结果和原有本地 Agent 都不会被删除。",
  codeEntries: ["src-tauri/src/extensions.rs", "src/components/ExtensionsPanel.tsx"],
};

export const AUTO_IMPORT_HELP: FeatureHelpContent = {
  title: "自动导入新增项",
  hint: "了解自动导入什么时候运行、会改什么",
  purpose: "应用启动时每周检查两次默认 Claude 中新增的 Skills 与 Agents，让共享库自动补齐新项目。",
  action: "只读取默认 Claude，只把共享库中还没有的完整项目复制成独立副本；同名项目直接跳过。",
  impact: "新增项目进入应用共享库后，会继续分发给所有未排除、未覆盖的目标。关闭后仍可手动导入。",
  userAction: "通常保持开启即可。可查看最近检查时间、新增数、跳过数和失败明细。",
  flow: ["应用启动并判断距离上次成功检查是否超过 3.5 天。", "只读扫描默认 Claude 的 Skills 与 Agents。", "校验后复制共享库缺少的完整项目。", "保存检查记录，再逐项更新目标。"],
  principles: [
    { title: "只拉不推", detail: "数据只从默认 Claude 流向应用共享库，应用绝不把共享内容写回默认 Claude。", status: "implemented" },
    { title: "只补缺", detail: "共享库已有同名项目时跳过，不在后台比较后覆盖，避免无提示改掉用户选择的版本。", status: "implemented" },
    { title: "失败会重试", detail: "有项目读取或复制失败时会记录原因，不推进成功时间；下次启动仍会尝试。", status: "implemented" },
  ],
  example: "默认 Claude 新增 lint-review 后，下一次到期检查会把它加入共享库；共享库已有 lint-review 时只记录跳过。",
  failure: "某个链接失效或文件无权限时，该项不会进入共享库，现有共享内容与各目标版本保持不变。",
  codeEntries: ["src-tauri/src/extensions.rs", "src/components/ExtensionsPanel.tsx"],
};

export const RESOURCE_OVERRIDE_HELP: FeatureHelpContent = {
  title: "环境覆盖与排除",
  hint: "了解目标自己的版本、排除和恢复共享版本",
  purpose: "允许某个目标使用自己的同名项目，或暂时不用某个共享项目，而不影响其他目标。",
  action: "覆盖会保留目标现有内容；排除只移除应用此前分发且尚未被修改的副本；恢复共享版本是唯一会主动替换目标同名内容的操作。",
  impact: "只影响当前选择的目标。共享库和其他环境不会被改动。",
  userAction: "看到“目标自己的版本”时无需处理；确实希望重新跟随共享库时再点“恢复共享版本”。",
  flow: ["读取共享内容、目标当前内容和上次分发记录。", "三者相同则继续继承。", "目标与上次记录不同则认定为本地覆盖。", "排除或恢复的明确选择写入分发记录。"],
  principles: [
    { title: "三方比较", detail: "同时比较共享版本、目标版本和应用上次写入的版本，从而识别是谁改了内容。", status: "implemented" },
    { title: "本地内容优先", detail: "无法证明内容仍由应用管理时就不覆盖，避免把用户在环境里的修改冲掉。", status: "implemented" },
    { title: "原子替换", detail: "先复制到临时位置并核对内容，再一次切换；中途失败时继续使用旧版本。", status: "implemented" },
  ],
  example: "test 环境改过 reviewer 后会显示“目标自己的版本”，corp 仍继续收到共享 reviewer 的更新。",
  failure: "目标文件无法读取时应用会报告错误并跳过，不会把读取失败当成用户删除。",
  codeEntries: ["src-tauri/src/extensions.rs"],
};

export const PLUGIN_RESULT_HELP: FeatureHelpContent = {
  title: "插件生效与重载",
  hint: "了解插件操作成功后何时能使用",
  purpose: "解释插件已经写入磁盘后，新的和正在运行的 Claude Code 会话如何加载它。",
  action: "每个目标环境的结果单独显示；成功的环境保留成功结果，不因另一环境失败而回滚。",
  impact: "新启动的会话直接读取新状态；已经运行的会话仍保留启动时加载的插件。",
  userAction: "安装、启停或卸载后，已打开的会话执行 /reload-plugins；更新插件或插件包含 monitors 时重启 Claude Code 会话。终端窗口本身不用关闭。",
  flow: ["Claude Code 官方命令完成安装或启停。", "应用重新读取目标环境的安装账本。", "界面逐环境显示真实结果。", "运行中的会话由用户按提示重新加载。"],
  principles: [
    { title: "不注入工作会话", detail: "应用不知道每个终端正在做什么，因此不会擅自向会话输入命令或中断任务。", status: "implemented" },
    { title: "逐环境结果", detail: "批量操作不是一个模糊的总结果，每个环境都有自己的成功或失败说明。", status: "implemented" },
  ],
  example: "corp 安装成功、test 网络失败时，corp 可在 /reload-plugins 后使用，test 会保留失败原因供重试；若执行的是更新，corp 需要重启会话。",
  failure: "官方命令超时、插件来源不存在或安装脚本失败时，只把该环境标为失败。",
  codeEntries: ["src-tauri/src/claude_cli.rs", "src-tauri/src/extensions.rs"],
};

export const AGENT_DEPENDENCY_HELP: FeatureHelpContent = {
  title: "Agent 缺少依赖",
  hint: "了解为什么 Agent 文件存在但仍不可用",
  purpose: "检查 Agent 声明要用的 Skill 和 MCP 在当前环境是否都存在。",
  action: "应用读取 Agent 配置区的 skills 与 tools 字段，只做检查并列出缺少项。",
  impact: "状态仅用于说明该 Agent 在所选环境能否完成设计任务，不会自动安装 Skill、MCP 或扩大工具权限。",
  userAction: "根据缺少项自行安装或配置；条件满足后刷新页面即可恢复为可用状态。",
  flow: ["解析 Agent 文件开头的配置区。", "到该环境的 Skills 目录检查引用。", "到该环境的 MCP 配置检查服务名。", "缺少任一项时显示具体原因。"],
  principles: [
    { title: "依赖只检查", detail: "Agent 声明依赖不等于授权应用安装依赖，检查与改变环境权限必须分开。", status: "implemented" },
    { title: "只读 Agent 工具声明", detail: "只分析 frontmatter 的 tools 字段，不从正文示例误判 MCP 依赖。", status: "implemented" },
  ],
  example: "reviewer 的 tools 包含 mcp__git__status，而 test 没有 git MCP 时，会显示“缺少 MCP：git”。",
  failure: "Agent 配置区损坏时导入会停止；已存在的共享版和环境版不会被覆盖。",
  codeEntries: ["src-tauri/src/extensions.rs"],
};

export const MCP_SCOPE_HELP: FeatureHelpContent = {
  title: "MCP 作用范围",
  hint: "了解所有环境、指定环境和当前项目的区别",
  purpose: "决定一项 MCP 配置写到哪里，以及哪些 Claude 会话能够使用。",
  action: "所有环境写入应用共享源并分发；指定环境只写该环境；当前项目写项目自己的配置。",
  impact: "范围越大，可见的环境越多。默认 Claude 始终只读，不会因为选择“所有环境”而被修改。",
  userAction: "公司通用服务选所有环境；某个网关专用服务选指定环境；团队仓库专用服务选当前项目。",
  flow: ["选择作用范围和目标。", "应用计算实际配置文件。", "预览将修改的条目。", "确认后原子写入并更新分发记录。"],
  principles: [
    { title: "范围对应真实落点", detail: "界面范围不是标签，它直接决定共享库、环境配置或项目配置这三个存储位置。", status: "implemented" },
    { title: "同名环境配置优先", detail: "指定环境已有自己的同名 MCP 时保留该版本，并显示它覆盖了共享配置。", status: "implemented" },
  ],
  example: "把 git 配为所有环境后 corp 和 test 都能使用；test 自己配置同名 git 时，只有 test 使用自己的地址。",
  failure: "配置文件损坏或预览版本过期时保存会中止，避免覆盖用户刚刚做出的修改。",
  codeEntries: ["src-tauri/src/mcp/", "src-tauri/src/shared_config.rs"],
};

export const MCP_HELP: FeatureHelpContent = {
  title: "MCP 服务",
  hint: "了解 MCP 的作用范围与共享规则",
  purpose: "让 Claude 连接文件、数据库、浏览器或公司服务等外部工具。MCP 本身不会思考，也不会主动分派任务。",
  action: "保存后，应用根据选择的范围写入共享源、指定环境或当前项目。共享项再单向发送到受管理环境。",
  impact: "“所有环境”影响每个受管理环境；“指定环境”只影响选中的环境；“当前项目”只在该项目中可用。",
  userAction: "如果环境已有同名配置，环境版本会优先并显示覆盖提示。需要统一时可主动恢复共享配置。",
  flow: [
    "界面把敏感值脱敏后展示，保存时保留完整配置。",
    "所有环境范围写入应用共享源，指定环境和项目写入各自文件。",
    "共享配置逐环境分发，只修改应用负责的条目。",
    "应用记录上次分发值，用来识别环境自己的修改。",
  ],
  principles: [
    {
      title: "单向分发",
      detail: "共享源只向环境发送配置，环境的修改不会反过来污染其他环境，也不会写回默认 Claude。",
      status: "implemented",
    },
    {
      title: "三方比较",
      detail: "应用同时比较共享值、环境当前值和上次分发值，从而区分正常更新、环境覆盖和用户删除。",
      status: "implemented",
    },
    {
      title: "原子写入与回退",
      detail: "先写完整临时文件再替换正式文件；本轮任一写入失败时恢复本轮已经改变的文件。",
      status: "implemented",
    },
  ],
  example: "例如共享 git 服务后，corp 自己修改了启动参数。下一次分发会保留 corp 的参数，并明确标成“已覆盖共享配置”。",
  failure: "配置文件损坏或无法读取时会中止分发，不会把读取失败误判成用户删除。",
  codeEntries: ["src-tauri/src/mcp/", "src-tauri/src/shared_config.rs", "src/components/mcp/McpPanel.tsx"],
};
