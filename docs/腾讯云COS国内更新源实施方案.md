# 腾讯云 COS 国内更新源实施方案

## 目标

在保留 GitHub Release 的同时，为 PathMux 增加一个中国大陆无需代理即可访问的更新源：

- 系统代理开启时，更新器自动遵循 Windows/macOS 系统代理；
- 系统代理关闭时，直接访问腾讯云 COS；
- 腾讯云 COS 作为主更新源，GitHub Release 作为备用源；
- Windows 和 macOS 继续使用同一套 Tauri 更新签名和发布流程；
- 腾讯云只保存公开的更新文件，不保存签名私钥、API Key 或用户配置。

## 前置准备

### 1. 创建 COS 存储桶

建议配置：

- 地域：`ap-guangzhou` 或 `ap-shanghai`；
- 存储类型：标准存储、单 AZ；
- 建立一个只用于 PathMux 更新文件的独立存储桶；
- 允许匿名读取公开更新文件，禁止匿名写入；
- 暂时不开启 CDN、全球加速、数据万象和跨区域复制；
- 设置费用预警和旧版本生命周期清理规则。

需要记录以下非敏感配置：

```text
COS_BUCKET=完整存储桶名称
COS_REGION=ap-guangzhou
COS_PUBLIC_BASE_URL=https://完整存储桶名称.cos.ap-guangzhou.myqcloud.com
```

### 2. 创建最小权限发布凭证

创建专用 CAM 子用户或 API 凭证，只允许 GitHub Actions 对该存储桶的 `pathmux/` 前缀执行发布所需的列举、读取、上传、覆盖和候选文件清理操作。

不要使用腾讯云主账号永久密钥，也不要把密钥写入代码、日志、Release 或本文档。

在 GitHub Actions Secrets 中配置：

```text
TENCENT_COS_SECRET_ID
TENCENT_COS_SECRET_KEY
TENCENT_COS_BUCKET
TENCENT_COS_REGION
TENCENT_COS_PUBLIC_BASE_URL
```

## COS 文件结构

```text
pathmux/
├── candidates/
│   └── v3.0.6/
│       └── latest.json
├── releases/
│   └── v3.0.6/
│       ├── macOS 自动更新产物
│       ├── Windows 自动更新产物
│       ├── macOS 手动安装包
│       └── Windows 手动安装包
└── stable/
    └── latest.json
```

不要根据扩展名猜测自动更新产物。发布脚本应读取 Tauri 生成的原始 `latest.json`，找出 `platforms.*.url` 实际引用的每个文件并逐一同步。

## 应用侧改造

### 1. 保留系统代理支持

`src-tauri/Cargo.toml` 中的 `reqwest` 必须持续启用 `system-proxy`。这样更新检查和更新包下载会自动遵循 Windows/macOS 系统代理设置，不需要写死 Clash 或代理端口。

### 2. 配置更新源顺序

在 `src-tauri/tauri.conf.json` 中把 COS 放在第一位、GitHub 放在第二位：

```json
"updater": {
  "endpoints": [
    "https://完整COS公开域名/pathmux/stable/latest.json",
    "https://github.com/Hollelihanqi/new-claude/releases/latest/download/latest.json"
  ],
  "pubkey": "保持现有更新签名公钥不变"
}
```

国内源必须排在第一位，避免无代理用户先等待 GitHub 连接超时。不要把腾讯云访问密钥放进应用；客户端读取的对象都应是公开更新文件。

## 国内版 latest.json

COS 上的 `latest.json` 必须保留原始版本号、发布日期、说明、平台键和签名内容，只把下载 URL 改成对应的 COS 地址。例如：

```json
{
  "version": "3.0.6",
  "notes": "版本说明",
  "pub_date": "2026-09-15T00:00:00Z",
  "platforms": {
    "darwin-universal": {
      "url": "https://COS域名/pathmux/releases/v3.0.6/实际macOS更新文件名",
      "signature": "原始签名内容"
    },
    "windows-x86_64": {
      "url": "https://COS域名/pathmux/releases/v3.0.6/实际Windows更新文件名",
      "signature": "原始签名内容"
    }
  }
}
```

平台键和文件名以构建产物为准，不应手工固定。COS 不参与签名，也不接触 `TAURI_SIGNING_PRIVATE_KEY`。

## GitHub Actions 改造

在现有 `.github/workflows/release.yml` 中加入两个阶段。

### 阶段 A：镜像预发布产物

在 Windows/macOS 构建完成、GitHub draft Release 已包含全部产物后：

1. 使用 `GH_TOKEN` 下载该 Tag 的 `latest.json` 和 Release 产物；
2. 校验 `.dmg`、`.msi`/`.exe`、`latest.json` 及清单实际引用的自动更新包齐全；
3. 读取 `platforms.*.url`，解析真实更新文件名；
4. 将版本化文件上传到 `pathmux/releases/v版本号/`；
5. 把清单中的 GitHub 下载 URL 改成 COS URL，签名字段原样保留；
6. 将改写后的清单先上传到 `pathmux/candidates/v版本号/latest.json`；
7. 对每个公开 COS URL 发起匿名 HTTPS 请求，确认返回成功且文件非空；
8. 任一上传或验证失败时终止发布，不能覆盖 `stable/latest.json`。

### 阶段 B：晋级稳定清单

GitHub Release 通过门禁并正式公开后：

1. 再次确认候选清单和所有版本化文件可下载；
2. 将候选清单复制或覆盖到 `pathmux/stable/latest.json`；
3. 下载稳定清单并校验版本号等于当前 Tag；
4. Internal Preview 和 Public Preview 使用独立目录，禁止覆盖 Stable 清单。

发布顺序必须是“版本化更新包先上传，稳定清单最后更新”。这样用户不会读到一个已经宣布新版本、但安装包尚不存在的清单。

## 缓存策略

建议为版本化文件设置长期不可变缓存：

```text
pathmux/releases/*
Cache-Control: public, max-age=31536000, immutable
```

稳定清单必须及时刷新：

```text
pathmux/stable/latest.json
Cache-Control: no-cache, max-age=0
```

## 安全与费用控制

- 发布凭证仅允许访问专用存储桶和 `pathmux/` 前缀；
- 存储桶公开读、私有写，绝不允许匿名上传；
- 设置账户余额和月费用预警；
- 初期直接使用 COS HTTPS 域名，不开 CDN；
- 稳定版保留最近 3～5 个版本，候选文件可在 90 天后清理；
- 当前稳定清单引用的文件不得自动删除；
- 对发布下载流量和异常突增设置监控，防止恶意刷流量；
- 每次发布保留 GitHub Release，作为审计记录和备用下载源。

## 验收清单

接入完成后必须在真实 Windows 和 macOS 上验证：

1. 关闭 Clash 和所有系统代理，可以从 COS 检查、下载并安装更新；
2. 开启系统代理，可以正常检查、下载并安装更新；
3. COS `latest.json` 中两个平台均指向 COS，而不是 GitHub；
4. COS 故障时，在 GitHub 可达的网络上可以尝试备用源；
5. 修改更新包任意字节后，应用因签名不匹配拒绝安装；
6. 安装包缺失、空文件或清单版本不一致时，发布工作流失败；
7. 没有新版本时显示“已是最新”，网络失败时显示真实失败信息；
8. 稳定版发布失败时，旧版 `stable/latest.json` 保持可用；
9. Windows 和 macOS 更新后都能正常重启并显示新版本号。

## 旧版本迁移

尚未包含 COS 地址的旧版本只能访问 GitHub。因此第一次迁移需要用户通过可用代理完成一次 GitHub 更新，或手动安装包含 COS 主源的新版本。完成这次迁移后，后续版本即可在没有代理时直接使用 COS。

## 下次实施顺序

1. 获取 COS 的存储桶名称、地域和公开基础地址；
2. 确认 GitHub Actions Secrets 已由仓库管理员配置；
3. 修改 Tauri updater endpoints；
4. 编写并测试 COS 上传及清单改写脚本；
5. 将镜像预发布与稳定晋级步骤接入 `release.yml`；
6. 先发布 Internal Preview 做双平台真机验证；
7. 验证通过后再发布 Stable，并观察第一周下载流量与费用。
