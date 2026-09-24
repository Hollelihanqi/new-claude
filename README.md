# PathMux

> **同一项目，多终端、多网关并行工作。**

PathMux 是面向 Claude Code 的桌面管理工具。在 Windows 和 macOS 上创建网关环境或独立登录环境，然后直接在终端中使用。个人工作与公司项目可以同时进行，各用各的配置。

## 为什么选择 PathMux

- **并行使用，互不干扰**：每次命令独立选择环境。同一项目中，不同终端可以同时连接不同网关；默认 Claude 保持原样。
- **配置一次，终端直用**：创建环境并接入终端后，用 `claude <环境名>` 启动。桌面应用无需一直开着。
- **管理更省心**：集中查看环境、MCP 服务与扩展，配合本地用量洞察和诊断工具，减少手动维护配置的麻烦。

```bash
claude          # 使用默认 Claude
claude corp     # 使用名为 corp 的环境启动
```

## 开始使用

1. 先安装 [Claude Code](https://code.claude.com/docs/en/setup)，确认终端可以运行 `claude`。
2. 从 [Releases](https://github.com/Hollelihanqi/new-claude/releases) 下载适合 Windows 或 macOS 的安装包。
3. 在 PathMux 的「环境」页面创建环境，点击「保存更改」，重新打开终端即可使用（Windows 使用 PowerShell）。

需要接入公司网关时，准备管理员提供的网关地址和 Key；如果网关使用自签名证书，可在「设置」中导入 CA 证书。更多操作说明见应用内「使用帮助」。

## 本地开发

项目基于 React、Tauri 和 Rust。准备好 Node.js、pnpm、Rust 及对应平台的 Tauri 构建依赖后：

```bash
pnpm install
pnpm tauri dev
```

构建安装包：`pnpm tauri build`。

## 开源许可

[MIT License](LICENSE)
