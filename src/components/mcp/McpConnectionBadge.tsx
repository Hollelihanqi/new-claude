import { Badge, Loader, Text, Tooltip } from "@mantine/core";
import { IconAlertCircle, IconCircleCheck, IconClock } from "@tabler/icons-react";
import type { McpConnectionCheck } from "../../api";

export default function McpConnectionBadge({
  enabled,
  supported,
  checks,
  busy,
  errors,
}: {
  enabled: boolean;
  supported: boolean;
  checks: McpConnectionCheck[];
  busy: boolean;
  errors: string[];
}) {
  if (!enabled) {
    return <Badge className="mcp-connection-badge" color="gray" variant="light">未检测</Badge>;
  }
  if (!supported) {
    return (
      <Tooltip label="项目级和指定环境配置会在对应项目会话中建立连接" multiline maw={280}>
        <Badge className="mcp-connection-badge" color="gray" variant="light">随会话检测</Badge>
      </Tooltip>
    );
  }
  if (busy && checks.length === 0) {
    return (
      <Badge className="mcp-connection-badge" color="gray" variant="light" leftSection={<Loader size={10} />}>
        检测中
      </Badge>
    );
  }

  const failed = checks.filter((check) => check.status === "failed");
  const pending = checks.filter((check) => check.status === "pending");
  const connected = checks.filter((check) => check.status === "connected");
  const unknown = checks.filter((check) => check.status === "unknown");
  const detailParts = ["这是独立健康检查，不代表已运行 Claude 会话的实时连接。"];
  detailParts.push(...checks.map(
    (check) => `${check.environment}：${connectionStatusLabel(check.status)}\n${check.detail}`
  ));
  detailParts.push(...errors);
  const details = detailParts.join("\n\n") || "尚未获得连接结果，请刷新重试";

  let badge;
  if (failed.length > 0) {
    badge = (
      <Badge className="mcp-connection-badge" color="red" variant="light" leftSection={<IconAlertCircle size={12} />}>
        {failed.length === 1 ? "连接失败" : `${failed.length} 个环境失败`}
      </Badge>
    );
  } else if (pending.length > 0) {
    badge = (
      <Badge className="mcp-connection-badge" color="orange" variant="light" leftSection={<IconClock size={12} />}>
        待授权
      </Badge>
    );
  } else if (errors.length > 0) {
    badge = (
      <Badge className="mcp-connection-badge" color="orange" variant="light" leftSection={<IconAlertCircle size={12} />}>
        检测异常
      </Badge>
    );
  } else if (connected.length > 0 && unknown.length === 0) {
    badge = (
      <Badge className="mcp-connection-badge" color="teal" variant="light" leftSection={<IconCircleCheck size={12} />}>
        可连接
      </Badge>
    );
  } else {
    badge = <Badge className="mcp-connection-badge" color="gray" variant="light">未确认</Badge>;
  }

  return (
    <Tooltip
      label={<Text size="xs" style={{ whiteSpace: "pre-line" }}>{details}</Text>}
      multiline
      maw={380}
    >
      {badge}
    </Tooltip>
  );
}

function connectionStatusLabel(status: McpConnectionCheck["status"]) {
  return {
    connected: "可连接",
    failed: "连接失败",
    pending: "待授权",
    unknown: "未确认",
  }[status];
}
