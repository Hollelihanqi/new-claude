# ChatGPT 启动与本地权限菜单排查

日期：2026-10-09。客户端安装包：26.1002.7124.0，内置应用版本：26.1002.52244。

用户已确认使用本地模式。本次仅排查启动环境和权限菜单显示条件，没有修改账号、批准策略或客户端权限设置。

## 结论与边界

**已确认 PathMux 的 Windows 启动方式丢失微软商店应用的包身份。** 这是一个实际兼容性缺陷。它是主程序经 PathMux 启动后出现异常的重要嫌疑，但尚未完成改变启动方式后的权限菜单对照验证，不能声称已经证明它是菜单消失的唯一原因。

另外，独立实例和主程序的菜单偏好不同。客户端在本地模式下也有“只剩一个可选权限模式时隐藏选择器”的逻辑。不能仅凭缺少选择器就断定权限系统没有运行，也不能通过强制开启自动审核来掩盖问题。

## 本机证据

1. 使用 Windows 原生 `GetPackageFullName` 读取正在运行的进程：系统启动的主程序有 `OpenAI.Codex_26.1002.7124.0_x64__2p2nqsd0c76g0` 身份；PathMux 启动的两个独立实例均返回无包身份。
2. 对应实例的官方启动日志均记录 `windows_runtime_framework_identity_unavailable`，原因是 `no-package-identity`。正常主程序没有这条告警。
3. 只读检查官方客户端启动代码：Windows 运行框架会检查当前包身份；缺失时无法启用相应的运行框架。这不是 PathMux 自己生成的错误提示。
4. 两个实例的官方日志都有成功的 `permissionProfile/list` 响应，未记录该请求失败。因此，“实例完全没有权限能力”的说法不符合现有证据。
5. 主程序保存的权限模式显示偏好是包含 `guardian-approvals: true`、`full-access: false` 的对象；两个实例保存的是旧式布尔值 `false`。客户端兼容解析中，布尔值 `false` 对应不显示 Full access，**不能把它直接解释为关闭自动审核或隐藏整个菜单**。
6. 官方客户端的选择器计算可选权限模式和自定义权限配置数量；在相应 Work 界面只剩一个选项时会不渲染选择器。是否触发这一条件，还取决于实例实际加载的功能开关和权限要求。

## PathMux 对应代码

- `src-tauri/src/chatgpt/process.rs::open_primary`：Windows 分支直接启动安装目录内的可执行文件，没有通过已注册的 Windows 应用入口启动。
- `src-tauri/src/chatgpt/process.rs::configure`、`profile_command`：独立实例通过环境变量和 `--user-data-dir` 隔离目录，但普通进程启动不会自动获得 MSIX 包身份。
- `src-tauri/src/chatgpt/storage.rs::config_text`、`create`：创建独立配置，未自动复制主程序的权限模式偏好。源码未写入上述 `composer-permission-mode-visibility` 偏好。
- 本次先前修复的 `reopen` 处理关闭窗口后的客户端恢复流程，解决的是另一条路径，不能视为已修复初次启动的包身份问题。

## 下一步应验证的修复方向

1. 微软商店版主程序使用 Windows 正式应用激活入口，并检查启动后包身份、账号和目录是否与系统启动一致。
2. 独立实例需要同时保证独立目录和官方运行环境。不能用仅供调试的包上下文命令冒充完整生产启动方案，也不能为获得包身份而合并实例数据或凭证。
3. 先对照“从系统启动 / 从 PathMux 启动”的本地权限菜单，再核对独立实例的 Settings → General → Permissions 和实际可用模式。修改配置前应完全退出对应实例，避免与客户端写入冲突。
4. 不自动开启 Approve for me 或 Full access，不修改组织规则，不把 UI 菜单隐藏解释为 API 模式必然不支持插件或权限。
5. Windows 应用包身份问题不适用于 macOS；任何启动改动仍应覆盖 macOS 的应用激活及独立目录行为。

## 官方资料

- [OpenAI：权限模式与首次启用](https://learn.chatgpt.com/docs/permission-modes)：额外权限模式需要在应用设置中启用，模式是否可用还受配置与组织要求约束。
- [Microsoft：Invoke-CommandInDesktopPackage](https://learn.microsoft.com/en-us/powershell/module/appx/invoke-commandindesktoppackage?view=windowsserver2025-ps)：该命令用于调试；文档明确不保证创建进程与真实应用进程的行为相同，因此本次未将它接入生产启动流程。
