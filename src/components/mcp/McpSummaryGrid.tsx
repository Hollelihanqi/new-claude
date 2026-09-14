import { Card, SimpleGrid, Text } from "@mantine/core";
import type { McpSummary } from "../../api";

export default function McpSummaryGrid({
  summary,
  sharedOverrideCount,
}: {
  summary?: McpSummary;
  sharedOverrideCount?: number;
}) {
  return (
    <SimpleGrid
      cols={{ base: 2, md: 4 }}
      className="mcp-summary-grid"
      aria-busy={!summary}
    >
      <SummaryCard label="全部定义" value={summary?.total} />
      <SummaryCard label="已启用" value={summary?.enabled} color="teal" />
      <SummaryCard label="存在警告" value={summary?.warnings} color="orange" />
      <SummaryCard
        label="环境覆盖"
        value={summary ? (sharedOverrideCount ?? 0) : undefined}
        color="gray"
        title="环境里的同名配置与共享配置不同，因此保留了该环境自己的值"
      />
    </SimpleGrid>
  );
}

function SummaryCard({
  label,
  value,
  color,
  title,
}: {
  label: string;
  value?: number;
  color?: string;
  title?: string;
}) {
  return (
    <Card withBorder padding="md" radius="md" title={title}>
      <Text size="xs" c="dimmed">{label}</Text>
      <Text fw={700} size="xl" c={color}>{value ?? "—"}</Text>
    </Card>
  );
}
