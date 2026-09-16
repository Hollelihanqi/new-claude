import { useState } from "react";
import { Alert, Button, Group, Stack, Text } from "@mantine/core";
import { IconAlertTriangle, IconTrash } from "@tabler/icons-react";
import type { McpDeadEntry, McpSourceIssue } from "../../api";
import RiskConfirm from "../RiskConfirm";

interface Props {
  issues: McpSourceIssue[];
  /** 可被一键清理安全删除的死条目（目录确认不存在） */
  deadEntries: McpDeadEntry[];
  /** 页面是否激活（控制 RiskConfirm 的 opened，防止后台页面弹出） */
  pageActive: boolean;
  busy: boolean;
  /** 执行清理；成功（应关弹窗）返回 true；失败返回 false（弹窗保持打开可重试） */
  onCleanup: () => Promise<boolean>;
}

/** 「配置来源问题」警告卡：列出全部来源问题，死条目（目录已不存在）提供一键清理。
 *  issues 负责"出了什么问题"，deadEntries 负责"哪些能清"——两份信息有意冗余，
 *  sourceId + 路径可互相关联。 */
export default function McpSourceIssuesCard({
  issues,
  deadEntries,
  pageActive,
  busy,
  onCleanup,
}: Props) {
  const [confirmOpen, setConfirmOpen] = useState(false);
  if (issues.length === 0) return null;
  const fileCount = new Set(deadEntries.map((e) => e.filePath)).size;

  return (
    <>
      <Alert color="orange" icon={<IconAlertTriangle size={16} />} title="配置来源问题">
        <Stack gap={2}>
          {issues.map((iss, i) => (
            <Text size="xs" key={i}>
              <Text span fw={600}>{iss.sourceId}</Text>
              {iss.path ? ` · ${iss.path}` : ""}：{iss.detail}
            </Text>
          ))}
          {deadEntries.length > 0 && (
            <Group justify="space-between" align="center" mt={6} wrap="nowrap">
              <Text size="xs" c="dimmed">
                其中 {deadEntries.length} 条指向已不存在的目录，可一键清理（清理前自动备份原文件）。
              </Text>
              <Button
                size="xs"
                variant="light"
                style={{ flexShrink: 0 }}
                leftSection={<IconTrash size={14} />}
                loading={busy}
                onClick={() => setConfirmOpen(true)}
              >
                一键清理死条目（{deadEntries.length}）
              </Button>
            </Group>
          )}
        </Stack>
      </Alert>
      <RiskConfirm
        opened={pageActive && confirmOpen}
        level="high"
        title="清理目录已不存在的死条目"
        consequences={[
          `将删除 ${deadEntries.length} 条记录（涉及 ${fileCount} 个配置文件）：各环境 .claude.json 里的失效项目键，以及项目登记表里的失效条目。`,
          "只删除「目录确认已不存在」的条目；目录仍存在但权限不足、路径非法或不是目录的一律保留。",
          "被改写的文件会先自动备份到 ~/.cc-manager/mcp-backups（每个来源保留最近 5 份），恢复需手动用备份覆盖。",
          "正在运行的 Claude Code 会话可能同时写 .claude.json，建议先关闭相关会话再清理。",
        ]}
        detail={deadEntries.map((e) => e.rawPath).join("；")}
        confirmLabel="确认清理"
        busy={busy}
        onCancel={() => setConfirmOpen(false)}
        onConfirm={async () => {
          // 失败保持弹窗打开，保留上下文让用户重试（与 CA 清空一致）
          if (await onCleanup()) setConfirmOpen(false);
        }}
      />
    </>
  );
}
