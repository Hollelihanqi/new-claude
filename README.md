# Claude Code 配置工具（Tauri 桌面应用）

一个跨平台（macOS / Windows）桌面 App：在一台电脑上管理多个 Claude Code 实例
（你的主账户 + 公司路由），日常在任意项目目录直接 `claude` / `claude corp`，
切换不掉线、不串配置。

- 界面：React + Mantine
- 外壳：Tauri 2（用系统自带 WebView，安装包小、省内存）
- 系统操作：Rust 后端（写 shell 配置、加密 token、建符号链接）

> **它只做配置。** 配好后退到幕后；日常使用是你自己的终端，跟平时一样。
> 应用内的「使用指南」标签页有面向新手的完整说明。

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
npm install
npm run tauri build
```

完成后安装包在：`src-tauri/target/release/bundle/` 里
（macOS 在 `dmg/`，Windows 在 `msi/` 和 `nsis/`）。

> 想先看看效果、不打包，可运行 `npm run tauri dev` 直接开发预览。

---

## 四、同事如何安装（重要：未签名的安全提示）

这个 App 没有买证书签名，所以同事首次打开会看到系统拦截，**这是正常的**，绕过即可：

- **macOS**：双击 `.dmg`，把 App 拖进「应用程序」。首次打开如果提示"无法验证开发者"，
  **右键点 App →「打开」→ 再点「打开」**。（或系统设置→隐私与安全性→仍要打开）
- **Windows**：双击 `.msi`/`.exe`。若弹出"Windows 已保护你的电脑"，
  点「更多信息」→「仍要运行」。

要彻底消除这些提示，需要购买开发者证书签名（mac 99 美元/年、Windows 证书若干），
内部小范围使用可以不签名，教同事点一下绕过即可。

---

## 五、装好之后怎么用

> **公司路由用户先做一次（重要）**：公司网关是自签名证书，必须先导入 CA 根证书，
> 否则 `claude corp` 连不上。向管理员要 `ca-cert.pem`，然后执行一次：
> - macOS：`sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain ca-cert.pem`
> - Windows（管理员 PowerShell）：`certutil -addstore Root ca-cert.pem`

1. 打开 App，切到「实例配置」。
2. 「新建」一个实例：名称填命令词（如 `corp`），类型选「自定义路由」，
   网关地址按公司说明填（通常带 `/anthropic` 后缀，例如 `https://10.0.7.83:8080/anthropic`），
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
├── src/                       React + Mantine 界面
│   ├── App.jsx                顶部状态 + 两个 Tab
│   ├── api.js                 调用 Rust 命令
│   └── components/
│       ├── ConfigPanel.jsx    实例配置
│       └── GuidePanel.jsx     使用指南
├── src-tauri/                 Rust 后端（系统操作）
│   ├── src/main.rs            全部命令实现
│   ├── tauri.conf.json        应用配置
│   ├── capabilities/          权限
│   ├── icons/                 各平台图标（已生成）
│   └── Cargo.toml
└── .github/workflows/release.yml   自动构建双平台包

提示：未在 Linux 容器内本地编译 Rust（目标是 mac/win）。
前端已通过 Vite 构建验证；Rust 端会在 GitHub Actions 或你本地首次构建时编译。
若遇报错把信息发来即可修。
```
