import {
  Stack,
  Card,
  Title,
  Text,
  SimpleGrid,
  ThemeIcon,
  Group,
  Timeline,
  Code,
  List,
  Alert,
} from "@mantine/core";
import {
  IconRoute,
  IconTerminal2,
  IconShieldLock,
  IconLink,
  IconBrandReact,
  IconChecklist,
  IconArrowDown,
  IconTrash,
  IconAlertTriangle,
} from "@tabler/icons-react";
import type { ReactNode } from "react";

function Tech({
  icon,
  name,
  desc,
}: {
  icon: ReactNode;
  name: string;
  desc: string;
}) {
  return (
    <Card withBorder radius="md" padding="md">
      <Group gap="sm" mb={6}>
        <ThemeIcon variant="light" size="lg" radius="md">
          {icon}
        </ThemeIcon>
        <Text fw={600}>{name}</Text>
      </Group>
      <Text size="sm" c="dimmed">
        {desc}
      </Text>
    </Card>
  );
}

export default function GuidePanel() {
  return (
    <Stack gap="lg">
      <Card withBorder radius="md" padding="lg">
        <Title order={4} mb="xs">
          这个工具解决什么问题？
        </Title>
        <Text size="sm">
          你有时用账户登录的 <Code>claude</Code>，有时要走公司的路由网关。以前来回切换
          要重新登录、对话还会断。这个工具让你在<b>任意项目目录</b>里直接敲命令——
          <Code>claude</Code> 用你的默认 Claude，<Code>claude corp</Code> 用公司路由——
          两者各用各的配置，互不打架，切换不掉线。
        </Text>
        <Text size="sm" mt="sm" c="dimmed">
          它<b>只负责配置</b>，配好后就退到幕后；日常使用是你自己的终端，跟平时一模一样。
        </Text>
      </Card>

      <Card withBorder radius="md" padding="lg">
        <Title order={4} mb="md">
          工作原理
        </Title>
        <Timeline active={3} bulletSize={28} lineWidth={2}>
          <Timeline.Item
            bullet={<IconTerminal2 size={16} />}
            title="你在项目目录里输入 claude corp"
          >
            <Text size="sm" c="dimmed">
              工作目录就是你当前所在的项目，不会被改变。
            </Text>
          </Timeline.Item>
          <Timeline.Item
            bullet={<IconRoute size={16} />}
            title="一个 shell 函数接管这一次调用"
          >
            <Text size="sm" c="dimmed">
              它只对这一次 claude 生效：临时把「家目录」指向独立文件夹，并设好公司路由地址和 token。
            </Text>
          </Timeline.Item>
          <Timeline.Item
            bullet={<IconArrowDown size={16} />}
            title="claude 在同一个目录里正常运行"
          >
            <Text size="sm" c="dimmed">
              因为家目录被换了，它读到的是这个环境独立的配置，不会污染你的默认 Claude。
            </Text>
          </Timeline.Item>
          <Timeline.Item
            bullet={<IconChecklist size={16} />}
            title="只影响这一次命令"
          >
            <Text size="sm" c="dimmed">
              你的普通 <Code>claude</Code> 完全不受影响。
            </Text>
          </Timeline.Item>
        </Timeline>
      </Card>

      <Card withBorder radius="md" padding="lg">
        <Title order={4} mb="md">
          用到了哪些技术
        </Title>
        <SimpleGrid cols={{ base: 1, sm: 2 }} spacing="md">
          <Tech
            icon={<IconBrandReact size={20} />}
            name="Tauri + React + Mantine"
            desc="桌面应用外壳与界面。Tauri 用系统自带的 WebView，所以打包后体积很小、运行很省内存。"
          />
          <Tech
            icon={<IconTerminal2 size={20} />}
            name="Shell 函数（zsh / bash / PowerShell）"
            desc="把 claude 包一层，识别到 corp 这类命令词时改用该环境启动。这是日常使用的核心，不依赖本应用运行。支持的终端见下方「终端支持范围」。"
          />
          <Tech
            icon={<IconShieldLock size={20} />}
            name="网关 Key 加密存储"
            desc="网关 Token 不存明文：macOS 放进钥匙串（Keychain），Windows 用 DPAPI 按当前用户加密。注意 DPAPI 是用户级保护——同一登录用户下的其他程序可以解密。"
          />
          <Tech
            icon={<IconLink size={20} />}
            name="共享与同步"
            desc="Skills 与 Agents 存在应用自己的共享库，按条目复制到每个环境；环境里的同名版本优先，也可以明确排除。Plugins 由应用调用 Claude Code 官方命令，在目标环境中独立安装和启停。MCP 从应用共享源单向分发，同名时保留环境配置并显示冲突。"
          />
        </SimpleGrid>
      </Card>

      <Card withBorder radius="md" padding="lg">
        <Title order={4} mb="xs">
          第一次使用需要什么
        </Title>
        <List size="sm" spacing="xs">
          <List.Item>
            <b>已安装 Claude Code</b>：终端里能直接运行 <Code>claude</Code>。
            这个工具是配置它的，没有它就没东西可跑。
          </List.Item>
          <List.Item>
            <b>（仅公司路由）导入一次 CA 证书</b>：公司网关是自签名证书，需按下方
            「导入 CA 证书」做一次，否则 <Code>claude corp</Code> 连不上。
          </List.Item>
          <List.Item>
            <b>就这些。</b>这个桌面应用本身不需要你额外装 Python、Node 或 Rust——
            那些只有「构建这个应用」的人才需要。你作为使用者，装好应用直接用。
          </List.Item>
        </List>
      </Card>

      <Card withBorder radius="md" padding="lg">
        <Title order={4} mb="xs">
          公司路由：导入一次 CA 证书（仅首次，必做）
        </Title>
        <Text size="sm" mb="sm">
          公司网关用的是自签名 HTTPS 证书。向管理员要 <Code>ca-cert.pem</Code>，
          在 App <b>顶栏点「CA 证书」→ 填入路径 → 导入</b>即可，否则{" "}
          <Code>claude corp</Code> 会因为"证书不被信任"而连不上。
        </Text>
        <Alert color="teal" variant="light" icon={<IconShieldLock size={16} />}>
          <Text size="sm">
            <b>不需要管理员权限，也不需要改系统信任库。</b>
            导入只作用于<b>网关环境</b>：仅在 <Code>claude &lt;环境名&gt;</Code> 那一次启动时注入
            （<Code>NODE_EXTRA_CA_CERTS</Code>），直接敲的 <Code>claude</Code> 与独立登录环境都不受影响。
            它不会改动 macOS 钥匙串或 Windows 系统根证书库 ——
            所以<b>不必</b>去执行 <Code>sudo security add-trusted-cert</Code> 或管理员的{" "}
            <Code>certutil -addstore Root</Code>。各处作用的信任范围见下方「终端支持范围」一节。
          </Text>
        </Alert>
      </Card>

      <Card withBorder radius="md" padding="lg">
        <Title order={4} mb="md">
          怎么配置和使用
        </Title>
        <Timeline active={4} bulletSize={26} lineWidth={2}>
          <Timeline.Item title="切到「环境配置」标签页">
            <Text size="sm" c="dimmed">
              点顶部的「环境配置」。
            </Text>
          </Timeline.Item>
          <Timeline.Item title="新建一个环境">
            <Text size="sm" c="dimmed">
              名称填命令词（如 <Code>corp</Code>），类型选「网关环境」，填公司网关地址和 token。
            </Text>
          </Timeline.Item>
          <Timeline.Item title="点「保存并接入终端」">
            <Text size="sm" c="dimmed">
              它会把一段 shell 函数写进你的终端配置（带标记、可移除）。
            </Text>
          </Timeline.Item>
          <Timeline.Item title="重开一个终端窗口">
            <Text size="sm" c="dimmed">
              让配置生效。只有改了配置才需要这一步。
            </Text>
          </Timeline.Item>
          <Timeline.Item title="开始用">
            <Code block>{`cd 任意项目目录\nclaude          # 默认 Claude\nclaude corp     # 公司路由`}</Code>
            <Text size="xs" c="dimmed" mt={6}>
              这两个命令只在「终端支持范围」里列出的终端、且集成已接入时才有效。
              诊断页会逐个终端显示「已接入 / 终端入口未生效」——显示未生效时，敲
              <Code>claude corp</Code> 不会使用该环境启动。
            </Text>
          </Timeline.Item>
        </Timeline>
      </Card>

      <Alert
        color="orange"
        variant="light"
        icon={<IconChecklist size={18} />}
        title="模型映射的两个注意点"
      >
        <List size="sm" spacing="xs">
          <List.Item>
            在环境会话里用 <Code>/model</Code> 时<b>只选档位别名</b>（Opus / Sonnet /
            Haiku / Default），不要选具体型号 ID——具体型号会写死进环境配置、绕过
            这里设置的模型映射。App 会在「环境配置」页检测到并提供一键还原。
          </List.Item>
          <List.Item>
            尽量<b>在项目目录里</b>启动 Claude。从用户主目录（~）启动时，默认 Claude通过{" "}
            <Code>/model</Code> 固定选择的型号可能会优先于环境中的模型映射。
          </List.Item>
        </List>
      </Alert>

      <Alert
        color="teal"
        variant="light"
        icon={<IconShieldLock size={18} />}
        title="关于安全"
      >
        <Text size="sm">
          网关 Token 不以明文保存：macOS 存进钥匙串，Windows 用 DPAPI 按当前用户加密。
          DPAPI 是用户级保护，不是「只有你能解密」——同一登录用户下的其他程序同样可以解密；
          Windows 上这份密文还会随配置备份一起导出到桌面。
          WorkBuddy 的 Key 与 MCP 的 env / header 密钥是明文文件，不在上述保护范围内。
          运行环境时 token 只作为那一次进程的环境变量存在，不写进命令行历史。
        </Text>
      </Alert>

      <Card withBorder radius="md" padding="lg">
        <Group gap="xs" mb="xs">
          <ThemeIcon variant="light" color="blue" radius="md">
            <IconTerminal2 size={18} />
          </ThemeIcon>
          <Title order={5}>终端支持范围</Title>
        </Group>
        <Text size="sm">
          <Code>claude 环境名</Code> 靠一层 shell 函数生效，所以只在这些终端里有效：
        </Text>
        <SimpleGrid cols={{ base: 1, sm: 2 }} mt="xs" spacing="xs">
          <div>
            <Text size="xs" fw={700} c="dimmed">
              macOS
            </Text>
            <Text size="sm">
              <Code>zsh</Code>（<Code>~/.zshrc</Code>）、<Code>bash</Code>（
              <Code>~/.bash_profile</Code> 与 <Code>~/.bashrc</Code>）
            </Text>
          </div>
          <div>
            <Text size="xs" fw={700} c="dimmed">
              Windows
            </Text>
            <Text size="sm">
              <Code>Windows PowerShell 5.1</Code> 与 <Code>PowerShell 7+</Code>
              （两者配置文件是两个不同路径，会分别接入、分别检测）
            </Text>
          </div>
        </SimpleGrid>
        <Alert color="gray" variant="light" mt="sm" icon={<IconAlertTriangle size={16} />}>
          <Text size="sm">
            <b>未支持</b>：<Code>cmd.exe</Code>、<Code>fish</Code>、<Code>sh</Code>、
            <Code>Git Bash</Code>、<Code>WSL</Code>。这些终端里敲 <Code>claude 环境名</Code>{" "}
            不会使用该环境启动，也不会报错 —— 会原样走默认 Claude。需要使用其他环境时请改用上面列出的终端。
            （cmd 需要遮蔽 Claude Code 官方启动器才能接管，风险高于收益，故不做。）
          </Text>
        </Alert>
      </Card>

      <Card withBorder radius="md" padding="lg">
        <Group gap="xs" mb="xs">
          <ThemeIcon variant="light" color="gray" radius="md">
            <IconTrash size={18} />
          </ThemeIcon>
          <Title order={5}>想撤销 / 卸载</Title>
        </Group>
        <Text size="sm">
          打开 macOS 的 <Code>~/.zshrc</Code> 或 Windows 的 PowerShell{" "}
          <Code>$PROFILE</Code>，删掉带 <Code># cc-manager-integration</Code>{" "}
          标记的那一行即可。配置文件在 <Code>~/.cc-manager/</Code>，删掉整个文件夹就彻底清除。
        </Text>
      </Card>
    </Stack>
  );
}
