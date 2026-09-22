import {
  Accordion,
  Badge,
  Card,
  Code,
  Group,
  SimpleGrid,
  Stack,
  Text,
  ThemeIcon,
  Title,
} from "@mantine/core";
import {
  IconChartLine,
  IconCircleCheck,
  IconKey,
  IconPlugConnected,
  IconRoute,
  IconSettings,
  IconShieldLock,
  IconStack2,
  IconStethoscope,
  IconTerminal2,
  IconTrash,
} from "@tabler/icons-react";
import type { ReactNode } from "react";
import type { EnvInfo } from "../api";

function StartStep({
  number,
  title,
  children,
}: {
  number: string;
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="guide-step">
      <span className="guide-step-number">{number}</span>
      <div>
        <Text fw={700} size="sm">{title}</Text>
        <Text size="sm" c="dimmed" mt={4}>
          {children}
        </Text>
      </div>
    </div>
  );
}

function Feature({
  icon,
  title,
  description,
}: {
  icon: ReactNode;
  title: string;
  description: string;
}) {
  return (
    <div className="guide-feature">
      <ThemeIcon variant="light" size={38} radius={12}>
        {icon}
      </ThemeIcon>
      <div>
        <Text fw={700} size="sm">
          {title}
        </Text>
        <Text size="xs" c="dimmed" mt={2}>
          {description}
        </Text>
      </div>
    </div>
  );
}

function CommandRow({ command, label }: { command: string; label: string }) {
  return (
    <div className="guide-command-row">
      <Code>{command}</Code>
      <Text size="sm" c="dimmed">
        {label}
      </Text>
    </div>
  );
}

export default function GuidePanel({ env = null }: { env?: EnvInfo | null }) {
  const platformUi = env?.platform_ui;
  return (
    <Stack className="guide-page" gap="md">
      <Card className="guide-hero" withBorder radius="lg" padding="xl">
        <Group gap="xs" mb="sm">
          <Badge variant="light" radius="xl" leftSection={<IconCircleCheck size={13} />}>
            3 分钟上手
          </Badge>
          <Text size="xs" c="dimmed">
            配置一次，之后直接在终端使用
          </Text>
        </Group>

        <Title order={3}>第一次使用，只需完成 3 步</Title>
        <Text className="guide-hero-description" size="sm" c="dimmed" mt={6}>
          PathMux 为不同 Claude 环境保留独立配置，切换环境时不会改变当前项目目录。
        </Text>

        <SimpleGrid className="guide-steps" cols={3} spacing="sm" mt="lg">
          <StartStep number="1" title="检查 Claude Code">
            在终端运行 <Code>claude</Code>，能正常启动即可。
          </StartStep>
          <StartStep number="2" title="创建环境">
            打开「环境」，填写环境名称、网关地址和 Key。
          </StartStep>
          <StartStep number="3" title="接入终端">
            保存并接入终端，然后重新打开一个终端窗口。
          </StartStep>
        </SimpleGrid>
      </Card>

      <SimpleGrid className="guide-primary-grid" cols={2} spacing="md">
        <Card className="guide-section-card" withBorder radius="lg" padding="lg">
          <Group gap="sm" mb="md">
            <ThemeIcon variant="light" size={38} radius={12}>
              <IconTerminal2 size={20} />
            </ThemeIcon>
            <div>
              <Title order={5}>日常使用</Title>
              <Text size="xs" c="dimmed">
                只需要记住两个命令
              </Text>
            </div>
          </Group>
          <div className="guide-command-list">
            <CommandRow command="claude" label="使用默认 Claude" />
            <CommandRow command="claude 环境名" label="使用指定环境，例如 claude corp" />
          </div>
          <Text className="guide-note" size="xs" c="dimmed" mt="sm">
            请在项目目录中运行。PathMux 只切换配置，不会改变当前目录。
          </Text>
        </Card>

        <Card className="guide-section-card" withBorder radius="lg" padding="lg">
          <Group gap="sm" mb="md">
            <ThemeIcon variant="light" size={38} radius={12}>
              <IconRoute size={20} />
            </ThemeIcon>
            <div>
              <Title order={5}>功能速览</Title>
              <Text size="xs" c="dimmed">
                需要什么，就去对应页面
              </Text>
            </div>
          </Group>
          <SimpleGrid cols={2} spacing="xs">
            <Feature icon={<IconSettings size={19} />} title="环境" description="创建和切换 Claude 环境" />
            <Feature icon={<IconStack2 size={19} />} title="扩展与 MCP" description="管理插件、能力和服务" />
            <Feature icon={<IconChartLine size={19} />} title="洞察" description="查看用量与模型趋势" />
            <Feature icon={<IconStethoscope size={19} />} title="诊断" description="检查终端接入与异常" />
          </SimpleGrid>
        </Card>
      </SimpleGrid>

      <Card className="guide-faq-card" withBorder radius="lg" padding="lg">
        <div className="guide-section-heading">
          <Title order={4}>常见问题</Title>
          <Text size="sm" c="dimmed" mt={3}>
            遇到对应问题时再展开查看
          </Text>
        </div>

        <Accordion variant="separated" radius="md" mt="md">
          <Accordion.Item value="gateway-certificate">
            <Accordion.Control icon={<IconKey size={19} />}>公司网关连接失败怎么办？</Accordion.Control>
            <Accordion.Panel>
              <Stack gap="xs">
                <Text size="sm">
                  如果公司网关使用自签名证书，请向管理员获取 <Code>ca-cert.pem</Code>，然后在
                  「设置 → CA 证书」中导入一次。
                </Text>
                {platformUi?.gateway_certificate_instruction && (
                  <Text size="sm" c="dimmed">{platformUi.gateway_certificate_instruction}</Text>
                )}
              </Stack>
            </Accordion.Panel>
          </Accordion.Item>

          <Accordion.Item value="terminal-support">
            <Accordion.Control icon={<IconPlugConnected size={19} />}>
              输入 claude 环境名后没有切换环境？
            </Accordion.Control>
            <Accordion.Panel>
              <Stack gap="xs">
                <Text size="sm">
                  先打开「诊断」查看终端是否显示“已接入”。修改环境后，需要重新打开终端窗口。
                </Text>
                {platformUi?.terminal_support_instruction && (
                  <Text size="sm" c="dimmed">{platformUi.terminal_support_instruction}</Text>
                )}
              </Stack>
            </Accordion.Panel>
          </Accordion.Item>

          <Accordion.Item value="model-mapping">
            <Accordion.Control icon={<IconRoute size={19} />}>模型映射没有生效？</Accordion.Control>
            <Accordion.Panel>
              <Stack gap="xs">
                <Text size="sm">
                  在 Claude 的 <Code>/model</Code> 中请选择 Opus、Sonnet、Haiku 或 Default，不要固定具体型号 ID。
                </Text>
                <Text size="sm" c="dimmed">
                  建议从项目目录启动 Claude。若环境页提示存在固定型号，可直接使用页面提供的一键还原。
                </Text>
              </Stack>
            </Accordion.Panel>
          </Accordion.Item>

          <Accordion.Item value="security">
            <Accordion.Control icon={<IconShieldLock size={19} />}>Key 和配置如何保存？</Accordion.Control>
            <Accordion.Panel>
              <Stack gap="xs">
                {platformUi?.credential_storage_instruction && (
                  <Text size="sm" c="dimmed">{platformUi.credential_storage_instruction}</Text>
                )}
              </Stack>
            </Accordion.Panel>
          </Accordion.Item>

          <Accordion.Item value="remove">
            <Accordion.Control icon={<IconTrash size={19} />}>如何撤销终端接入或彻底清理？</Accordion.Control>
            <Accordion.Panel>
              <Stack gap="xs">
                {platformUi?.cleanup_instruction && (
                  <Text size="sm" c="dimmed">{platformUi.cleanup_instruction}</Text>
                )}
              </Stack>
            </Accordion.Panel>
          </Accordion.Item>
        </Accordion>
      </Card>
    </Stack>
  );
}
