# PathMux（Tauri 桌面应用）

> **每个终端，一条独立模型通道。** 同一项目里多个终端可同时连不同网关，互不干扰；
> 没有全局"当前环境"这回事 —— 每次启动命令自己决定用哪个环境。

一个跨平台（macOS / Windows）桌面 App：在一台电脑上管理多个 Claude Code 环境
（你的主账户 + 公司路由），日常在任意项目目录直接 `claude` / `claude corp`，
切换不掉线、不串配置。

- 界面：React + Mantine
- 外壳：Tauri 2（用系统自带 WebView，安装包小、省内存）
- 系统操作：Rust 后端（写 shell 配置、加密 token、建符号链接）

> **它只做配置。** 配好后退到幕后；日常使用是你自己的终端，跟平时一样。
> 应用内的「使用指南」标签页有面向新手的完整说明。

**许可**：[MIT](LICENSE) —— 可自由使用、修改、再分发，允许商业用途。欢迎二次开发。

---

## 一、给谁用、要装什么

- **使用者（你的同事）**：只需装好这个 App 的安装包，再加一个前提——
  电脑上已装 Claude Code（终端能运行 `claude`）。**不需要** Rust / Node / Python。
- **构建者（你）**：见下面两条路线，二选一。

---

## 二、构建出安装包：路线 A —— GitHub Actions（推荐，自动出双平台）

适合你和同事系统不一样（要同时出 mac 和 Windows 包）的情况。你本地什么都不用装。

1. 注册 GitHub，新建一个仓库（可设为 Private）。
2. 把本项目所有文件上传上去（网页端拖拽上传，或用 Git）。
3. 打一个版本 tag 触发自动构建。网页端操作：进仓库 →「Releases」→「Draft a new release」
   →「Choose a tag」输入 `v1.0.0` →「Create new tag」→ 发布。
   （或本地：`git tag v1.0.0 && git push origin v1.0.0`）
4. 进仓库的「Actions」标签，看到构建任务在 macOS 和 Windows 上各跑一次
   （首次约 10–20 分钟）。完成后「Releases」里会出现一个**草稿 Release**。
5. 编辑该 Release →「Publish」。里面就有：
   - macOS：`.dmg`
   - Windows：`.msi` 和 `.exe`
6. 把对应链接发给同事即可。

> CI 配置在 `.github/workflows/release.yml`，已经配好同时出两个平台、自动建 Release。

---

## 三、构建出安装包：路线 B —— 本地构建

只能构建**你当前这台电脑所属平台**的包（mac 上出 mac 包，Windows 上出 Windows 包）。

### 先装好环境（仅构建者需要，一次性）

通用：

- **Node.js 20+**（推荐 22 LTS）：https://nodejs.org
- **Rust**：https://www.rust-lang.org/tools/install

macOS 额外：

```bash
xcode-select --install
```

Windows 额外：

- Visual Studio 2022 生成工具，勾选「使用 C++ 的桌面开发」工作负载 + Windows SDK
- WebView2（Win10 1803+/Win11 已自带，老系统去微软官网装 Evergreen Bootstrapper）

### 构建命令

```bash
pnpm install
pnpm tauri build
```

完成后安装包在：`src-tauri/target/release/bundle/` 里
（macOS 在 `dmg/`，Windows 在 `msi/` 和 `nsis/`）。

> 想先看看效果、不打包，可运行 `pnpm tauri dev` 直接开发预览。

---

## 四、安装说明（重要：本应用未做代码签名）

本应用未购买代码签名证书，因此首次打开时系统会提示「无法验证开发者」或类似信息，**这是正常的**，手动放行一次即可：

- **macOS（Sequoia 15 及以上）**：双击 `.dmg`，把 App 拖进「应用程序」。首次打开会提示无法验证开发者——
  **右键打开的方式自 Sequoia 15 起已被 Apple 移除，不再有效**。请到
  **系统设置 → 隐私与安全性**，在下方找到该应用的提示，点 **「仍要打开」**，再输入密码确认。
- **macOS（Sonoma 14 及更早）**：可在 Finder 中右键点 App →「打开」→ 再点「打开」；
  若无效，同样走上面的「系统设置 → 隐私与安全性 → 仍要打开」。
- **Windows**：双击 `.msi`/`.exe`。若弹出"Windows 已保护你的电脑"，
  点「更多信息」→「仍要运行」。

---

## 五、装好之后怎么用

> **在脚本里判断 `claude` 是否成功 —— PowerShell 用户必读**
>
> 包装器承诺的是**参数与原生退出码**透传，**不是 PowerShell 的全部状态语义**。
> PowerShell 的函数包装模式下，请在命令结束后**立即**使用 `$LASTEXITCODE` 判断执行结果；
> **`$?` 不保证反映 Claude 的退出状态** —— 这是 PowerShell 的机制限制：
> 能改变调用方 `$?` 的只有 `$PSCmdlet.WriteError`，而它要求 advanced function，
> 那会破坏参数透传（实测 `claude corp` 直接报绑定错误、`claude -p hi` 两个参数被吞）。
> 该行为由回归测试 `generated_ps1_propagates_exit_code_and_restores_environment` 覆盖。
>
> 自动化脚本需要向外传递退出码时，请显式执行 `exit $LASTEXITCODE`。
>
> bash / zsh 不受此限制：`$?` 与退出码都按常规语义透传。

> **公司路由用户先做一次（重要）**：公司网关用自签名证书时，`claude corp` 会因证书校验失败连不上。
> 向管理员要 `ca-cert.pem`，然后在 App 顶栏点「CA 证书」导入即可。
>
> **不需要管理员权限，也不需要改系统信任库。** 导入只作用于**网关环境**：
> 只有 `claude <环境名>` 这一次启动会带上它（`NODE_EXTRA_CA_CERTS`），
> 直接敲的 `claude` 与独立登录环境都不受影响。它不会改动 macOS 钥匙串或 Windows 系统根证书库 ——
> 因此也不必为了这一步去执行 `sudo security add-trusted-cert` 或管理员 `certutil -addstore Root`。
> 各处作用的信任范围见下方「WorkBuddy 公司网关模型」一节。

1. 打开 App，切到「环境配置」。
2. 「新建」一个环境：名称填命令词（如 `corp`），类型选「路由环境」，
   网关地址按公司说明填（通常带 `/anthropic` 后缀，例如 `https://gateway.example.com:8080/anthropic`），
   API Key 填公司发给你的 `gw-sk-...`。
3. 点「保存并接入终端」。
4. **重开一个终端窗口**（mac 用任意终端 / Windows 用 **PowerShell**）。
5. 在任意项目目录：
   ```
   claude          # 主账户，原样
   claude corp     # 公司路由，同一目录，跑完自动恢复
   ```

只有改了配置才需要重开一次终端，之后正常用，不用再管。
App 内「使用指南」标签页有更详细的图文说明。

### WorkBuddy 公司网关模型

WorkBuddy 配置与 Claude Code 环境完全独立。打开左侧「WorkBuddy」，填写模型 ID、
MaaS Gateway 的 WorkBuddy 网关根地址和员工 Key，点击「保存并接入」。
应用会安全合并写入 WorkBuddy 实际读取的 `~/.workbuddy/models.json`，保留已有模型和
未知字段；WorkBuddy 通常会在 1 秒内热加载，随后可在它的自定义模型列表中直接选择。

网关地址请向你的管理员索取。**本应用不预置任何网关地址**，输入框只给一个示例格式。

- 网关用自签名证书时，先向管理员要 CA 根证书。**信任生效在哪一层，取决于你从哪儿导入**：
  - 顶栏「CA 证书」→ 只影响**网关环境**：仅在 `claude <环境名>` 那一次启动时注入
    （`NODE_EXTRA_CA_CERTS`），直接敲的 `claude` 与独立登录环境不受影响。
    **不改动系统信任库**；macOS 上也不会碰系统钥匙串。
  - 「WorkBuddy → 导入 CA」→ 写入 WorkBuddy 安装目录下的共享 `ca.pem`；
    在 Windows 上还会加入**当前登录用户**的「受信任根证书」库（不改动其他用户账户）。
- 是否需要自签名 CA 取决于网关怎么签的证书，与端口号无关。
- 「测试调用」会真实请求一次模型，可能产生少量用量并进入公司审计。
- 员工 Key 按 WorkBuddy 官方配置格式保存在其本地 `models.json`，不会写入 Claude Code 配置。
- 保存采用 revision 校验、备份和原子替换；检测到配置损坏或被其他程序并发修改时拒绝覆盖。

---

## 六、卸载 / 撤销

- 删掉 macOS `~/.zshrc`（和 `~/.bashrc`）或 Windows PowerShell `$PROFILE` 里
  带 `# cc-manager-integration` 标记的那一行。
- 配置在 `~/.cc-manager/`，删掉整个文件夹即彻底清除。
- App 本身按系统正常方式卸载即可。

---

## 七、项目结构

```
cc-switch/
├── index.html / vite.config.js / package.json   前端入口与依赖
├── src/                       React + Mantine 界面（TypeScript）
│   ├── App.tsx                五大工作区导航 + 环境状态 + 自动更新
│   ├── api.ts                 调用 Rust 命令（类型与后端一一对应）
│   └── components/
│       ├── ConfigPanel.tsx    环境配置（含模型钉死告警 + 一键还原）
│       ├── ExtensionsPanel.tsx Skills / Plugins / MCP / Agents 扩展总览
│       ├── UsagePanel.tsx     用量统计
│       ├── DiagnosticsPanel.tsx 健康检查、同步修复与诊断导出
│       ├── SettingsPanel.tsx  更新、证书、备份、主题与安全策略
│       ├── GuidePanel.tsx     使用指南
│       └── CaCertButton.tsx   CA 证书管理
├── src-tauri/                 Rust 后端（系统操作）
│   ├── src/main.rs            环境/集成/证书/用量等命令
│   ├── src/health.rs          健康检查、模型钉死检测、诊断导出
│   ├── src/sync.rs            共享链接与 MCP/插件启用状态合并同步
│   ├── src/claude_cli.rs      Claude CLI 定位与环境探测
│   ├── tauri.conf.json        应用配置
│   ├── capabilities/          权限
│   ├── icons/                 各平台图标（已生成）
│   └── Cargo.toml
├── .github/workflows/ci.yml        每次 push/PR 跑前端构建 + Rust 单测
└── .github/workflows/release.yml   打 tag 自动构建双平台包

提示：未在 Linux 容器内本地编译 Rust（目标是 mac/win）。
前端已通过 Vite 构建验证；Rust 端会在 GitHub Actions 或你本地首次构建时编译。
若遇报错把信息发来即可修。
```
