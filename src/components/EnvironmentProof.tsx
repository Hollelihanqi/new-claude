import { useEffect, useState } from "react";
import { Badge, Card, Code, Divider, Group, Stack, Text } from "@mantine/core";
import { api } from "../api";
import type { HealthItem, Profile, ProfileRuntimeInfo } from "../api";

/**
 * 环境证明卡（P1-1）。
 *
 * 把散落在各页的信息汇成**一处可核对的结论**：身份、路由、配置目录、模型档位、
 * Shell 接入状态、最近一次完整验证。目的是同事报障时能先看这一张卡，
 * 而不是在四个页面之间来回找。
 *
 * **只从已有 API 取数，不解析健康项的文案** —— 从 detail 里抠字串判断状态太脆，
 * 文案一改就静默失效。Shell 那部分用的是健康项的 **id**（`shell:<label>`）与 status，
 * 那是结构化字段。
 */

/** 相对时间。项目里不引第三方日期库，保持同一做法。 */
export function relativeTime(atSeconds: number, now = Date.now()): string {
  const delta = now - atSeconds * 1000;
  // 时间戳来自未来（系统时钟被改过）时不显示"负数分钟前"
  if (delta < 0) return "刚刚";
  const minutes = Math.floor(delta / 60_000);
  if (minutes < 1) return "刚刚";
  if (minutes < 60) return `${minutes} 分钟前`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} 小时前`;
  return `${Math.floor(hours / 24)} 天前`;
}

const STATUS_COLOR: Record<string, string> = { ok: "teal", warn: "yellow", fail: "red" };
const STATUS_TEXT: Record<string, string> = { ok: "正常", warn: "需注意", fail: "未生效" };

export default function EnvironmentProof({ items }: { items: HealthItem[] }) {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [runtime, setRuntime] = useState<ProfileRuntimeInfo[]>([]);
  const [verification, setVerification] = useState<{ at: number; problems: number } | null>(null);

  useEffect(() => {
    void api.listProfiles().then(setProfiles).catch(() => setProfiles([]));
    void api.profileRuntimeInfo().then(setRuntime).catch(() => setRuntime([]));
    void api.lastVerification().then(setVerification).catch(() => setVerification(null));
  }, [items]);

  // Shell 接入状态来自结构化字段（id 前缀 + status），不是解析文案
  const shells = items.filter((item) => item.id.startsWith("shell:"));
  const routers = profiles.filter((profile) => profile.type === "router");
  const configDirOf = (name: string) => runtime.find((r) => r.name === name)?.configDir;

  const modelSlots = (profile: Profile) =>
    [
      ["opus", profile.opusModel],
      ["sonnet", profile.sonnetModel],
      ["haiku", profile.haikuModel],
    ] as const;

  return (
    <Card withBorder padding="lg" radius="lg">
      <Group justify="space-between" mb="sm">
        <div>
          <Text fw={700}>环境证明</Text>
          <Text size="xs" c="dimmed">
            当前环境的可核对结论；每一条都来自实际检测，不是推断。
          </Text>
        </div>
        <Badge variant="light" color={verification ? (verification.problems ? "orange" : "teal") : "gray"}>
          {verification
            ? `最近验证：${relativeTime(verification.at)}${verification.problems ? `（${verification.problems} 项待处理）` : "（全部正常）"}`
            : "尚未验证"}
        </Badge>
      </Group>

      <Stack gap="xs">
        {/* Shell 接入：逐个终端分别列出 */}
        <div>
          <Text size="xs" fw={700} c="dimmed">
            终端入口
          </Text>
          {shells.length === 0 ? (
            <Text size="sm" c="dimmed">
              尚未检测（点上方「重新检测」）
            </Text>
          ) : (
            shells.map((shell) => (
              <Group key={shell.id} gap={6} wrap="nowrap">
                <Badge size="xs" variant="light" color={STATUS_COLOR[shell.status] ?? "red"}>
                  {STATUS_TEXT[shell.status] ?? "未知"}
                </Badge>
                <Text size="sm">{shell.label.replace(/^终端入口（|）$/g, "")}</Text>
              </Group>
            ))
          )}
        </div>

        <Divider my={4} />

        {routers.length === 0 ? (
          <Text size="sm" c="dimmed">
            还没有「网关环境」环境。
          </Text>
        ) : (
          routers.map((profile) => (
            <div key={profile.name}>
              <Group gap={6} mb={2}>
                <Badge size="xs" variant="light" color="blue">
                  路由
                </Badge>
                <Text size="sm" fw={600}>
                  {profile.name}
                </Text>
                {!profile.hasToken && (
                  <Badge size="xs" variant="light" color="red">
                    缺 API Key
                  </Badge>
                )}
              </Group>
              <Text size="xs" c="dimmed" style={{ wordBreak: "break-all" }}>
                路由地址：<Code>{profile.baseUrl || "<未配置>"}</Code>
              </Text>
              <Text size="xs" c="dimmed" style={{ wordBreak: "break-all" }}>
                配置目录：<Code>{configDirOf(profile.name) ?? "<未知>"}</Code>
              </Text>
              <Text size="xs" c="dimmed">
                模型档位：
                {modelSlots(profile)
                  .map(([slot, model]) => `${slot}=${model || "默认"}`)
                  .join("  ")}
              </Text>
            </div>
          ))
        )}
      </Stack>
    </Card>
  );
}
