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
          <Code>claude</Code> 用你的主账户，<Code>claude corp</Code> 用公司路由——
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
              因为家目录被换了，它读到的是这个实例独立的配置，不会污染你的主账户。
            </Text>
          </Timeline.Item>
          <Timeline.Item
            bullet={<IconChecklist size={16} />}
            title="命令结束，环境自动还原"
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
            name="Shell 函数（zsh / PowerShell）"
            desc="把 claude 包一层，识别到 corp 这类命令词时临时切换环境。这是日常使用的核心，不依赖本应用运行。"
          />
          <Tech
            icon={<IconShieldLock size={20} />}
            name="钥匙串 / DPAPI 加密"
            desc="Token 不存明文：macOS 放进钥匙串（Keychain），Windows 用 DPAPI 加密，只有你本人能解密。"
          />
          <Tech
            icon={<IconLink size={20} />}
            name="共享与同步"
            desc="所有实例自动与主账户共享 skills / plugins / agents / commands（目录联结）；MCP 与插件启用状态在每次启动 claude 时自动合并同步。跨实例共享的 MCP 请用 claude mcp add -s user 安装（默认作用域是项目级，不参与同步）。"
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
          公司路由：先导入 CA 证书（仅首次，必做）
        </Title>
        <Text size="sm" mb="sm">
          公司网关用的是自签名 HTTPS 证书。必须先把管理员给你的{" "}
          <Code>ca-cert.pem</Code> 导入系统信任，否则 <Code>claude corp</Code>{" "}
          会因为"证书不被信任"而连不上。每台机器只需做一次。
        </Text>
        <Text size="sm" fw={500}>
          macOS（终端，会要求输入开机密码）
        </Text>
        <Code block>
          sudo security add-trusted-cert -d -r trustRoot -k
          /Library/Keychains/System.keychain ca-cert.pem
        </Code>
        <Text size="sm" fw={500} mt="sm">
          Windows（以管理员身份打开 PowerShell）
        </Text>
        <Code block>certutil -addstore Root ca-cert.pem</Code>
        <Text size="xs" c="dimmed" mt="sm">
          把命令末尾的 ca-cert.pem 换成证书文件的实际路径，或先 cd 到证书所在的文件夹再运行。
        </Text>
      </Card>

      <Card withBorder radius="md" padding="lg">
        <Title order={4} mb="md">
          怎么配置和使用
        </Title>
        <Timeline active={4} bulletSize={26} lineWidth={2}>
          <Timeline.Item title="切到「实例配置」标签页">
            <Text size="sm" c="dimmed">
              点顶部的「实例配置」。
            </Text>
          </Timeline.Item>
          <Timeline.Item title="新建一个实例">
            <Text size="sm" c="dimmed">
              名称填命令词（如 <Code>corp</Code>），类型选「自定义路由」，填公司网关地址和 token。
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
            <Code block>{`cd 任意项目目录\nclaude          # 主账户\nclaude corp     # 公司路由`}</Code>
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
            在实例会话里用 <Code>/model</Code> 时<b>只选档位别名</b>（Opus / Sonnet /
            Haiku / Default），不要选具体型号 ID——具体型号会写死进实例配置、绕过
            这里设置的模型映射。App 会在「实例配置」页检测到并提供一键还原。
          </List.Item>
          <List.Item>
            尽量<b>在项目目录里</b>启动 Claude。从用户主目录（~）启动时，主账户通过{" "}
            <Code>/model</Code> 固定选择的型号可能会优先于空间中的模型映射。
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
          Token 从不以明文保存：macOS 存进钥匙串，Windows 用 DPAPI 加密（仅你本人可解密）。
          运行实例时 token 只作为那一次进程的环境变量存在，不写进命令行历史。
        </Text>
      </Alert>

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
