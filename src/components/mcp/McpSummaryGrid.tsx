import { Card, SimpleGrid, Text } from "@mantine/core";
import type { McpSummary } from "../../api";

export default function McpSummaryGrid({ summary }: { summary?: McpSummary }) {
  return (
    <SimpleGrid
      cols={{ base: 2, md: 4 }}
      className="mcp-summary-grid"
      aria-busy={!summary}
    >
      <SummaryCard label="全部定义" value={summary?.total} />
      <SummaryCard label="已启用" value={summary?.enabled} color="teal" />
      <SummaryCard label="存在警告" value={summary?.warnings} color="orange" />
      <SummaryCard label="被覆盖" value={summary?.shadowed} color="gray" />
    </SimpleGrid>
  );
}

function SummaryCard({
  label,
  value,
  color,
}: {
  label: string;
  value?: number;
  color?: string;
}) {
  return (
    <Card withBorder padding="md" radius="md">
      <Text size="xs" c="dimmed">{label}</Text>
      <Text fw={700} size="xl" c={color}>{value ?? "—"}</Text>
    </Card>
  );
}
