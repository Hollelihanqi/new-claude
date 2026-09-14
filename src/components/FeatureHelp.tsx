import { useState } from "react";
import {
  ActionIcon,
  Badge,
  Box,
  Code,
  Divider,
  Drawer,
  Group,
  Stack,
  Text,
  ThemeIcon,
  Timeline,
  Tooltip,
} from "@mantine/core";
import { IconCode, IconHelpCircle, IconRoute, IconShieldCheck } from "@tabler/icons-react";

export type FeatureStatus = "implemented" | "planned" | "verification";

export interface FeaturePrinciple {
  title: string;
  detail: string;
  status?: FeatureStatus;
}

export interface FeatureHelpContent {
  title: string;
  hint: string;
  purpose: string;
  action: string;
  impact: string;
  userAction: string;
  flow: string[];
  principles: FeaturePrinciple[];
  example: string;
  failure: string;
  codeEntries: string[];
}

const STATUS: Record<FeatureStatus, { label: string; color: string }> = {
  implemented: { label: "已实现", color: "teal" },
  planned: { label: "计划支持", color: "blue" },
  verification: { label: "待真机验证", color: "orange" },
};

export default function FeatureHelp({ content }: { content: FeatureHelpContent }) {
  const [opened, setOpened] = useState(false);

  return (
    <>
      <Tooltip label={content.hint} withArrow>
        <ActionIcon
          variant="subtle"
          color="gray"
          size="sm"
          radius="xl"
          aria-label={`了解${content.title}的工作原理`}
          onClick={() => setOpened(true)}
        >
          <IconHelpCircle size={17} />
        </ActionIcon>
      </Tooltip>

      <Drawer
        opened={opened}
        onClose={() => setOpened(false)}
        position="right"
        size="lg"
        title={`它是怎么工作的 · ${content.title}`}
        overlayProps={{ backgroundOpacity: 0.2, blur: 1 }}
        returnFocus
      >
        <Stack gap="lg">
          <Box>
            <Text fw={700} mb={8}>使用说明</Text>
            <Stack gap={8}>
              <HelpLine label="它能做什么" value={content.purpose} />
              <HelpLine label="操作后会发生什么" value={content.action} />
              <HelpLine label="会影响哪些地方" value={content.impact} />
              <HelpLine label="你还需要做什么" value={content.userAction} />
            </Stack>
          </Box>

          <Divider />

          <Box>
            <Group gap={8} mb="sm">
              <ThemeIcon variant="light" size="md"><IconRoute size={16} /></ThemeIcon>
              <Text fw={700}>数据怎样流动</Text>
            </Group>
            <Timeline active={content.flow.length} bulletSize={22} lineWidth={2}>
              {content.flow.map((step, index) => (
                <Timeline.Item key={`${index}-${step}`} bullet={index + 1}>
                  <Text size="sm">{step}</Text>
                </Timeline.Item>
              ))}
            </Timeline>
          </Box>

          <Box>
            <Group gap={8} mb="sm">
              <ThemeIcon variant="light" color="teal" size="md"><IconShieldCheck size={16} /></ThemeIcon>
              <Text fw={700}>设计原理</Text>
            </Group>
            <Stack gap="sm">
              {content.principles.map((item) => {
                const status = item.status ? STATUS[item.status] : undefined;
                return (
                  <Box key={item.title} p="sm" className="feature-help-principle">
                    <Group justify="space-between" gap="xs" mb={4}>
                      <Text size="sm" fw={650}>{item.title}</Text>
                      {status && <Badge size="xs" variant="light" color={status.color}>{status.label}</Badge>}
                    </Group>
                    <Text size="sm" c="dimmed">{item.detail}</Text>
                  </Box>
                );
              })}
            </Stack>
          </Box>

          <Box>
            <Text fw={700} mb={6}>实际示例</Text>
            <Text size="sm">{content.example}</Text>
            <Text size="sm" c="orange.8" mt={8}>{content.failure}</Text>
          </Box>

          <Box>
            <Group gap={8} mb="xs">
              <IconCode size={17} />
              <Text fw={700}>主要代码入口</Text>
            </Group>
            <Text size="xs" c="dimmed" mb={8}>
              这里列稳定的文件入口，不写容易随改动失效的固定行号。
            </Text>
            <Stack gap={5}>
              {content.codeEntries.map((path) => <Code key={path} block>{path}</Code>)}
            </Stack>
          </Box>
        </Stack>
      </Drawer>
    </>
  );
}

function HelpLine({ label, value }: { label: string; value: string }) {
  return (
    <Box>
      <Text size="xs" fw={700} c="dimmed">{label}</Text>
      <Text size="sm">{value}</Text>
    </Box>
  );
}
