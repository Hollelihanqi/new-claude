import { useState } from "react";
import { Alert, Badge, Button, Card, Group, SegmentedControl, SimpleGrid, Stack, Text, ThemeIcon, Title } from "@mantine/core";
import { IconArchive, IconCertificate, IconDownload, IconLock, IconPalette, IconShieldCheck } from "@tabler/icons-react";
import { api } from "../api";
import type { EnvInfo } from "../api";
import CaCertButton from "./CaCertButton";

type Scheme = "a" | "b";

export default function SettingsPanel({ env, scheme, setScheme, appVersion, onCheckUpdate, onEnvironmentChanged }: { env: EnvInfo | null; scheme: Scheme; setScheme: (value: Scheme) => void; appVersion: string; onCheckUpdate: () => void; onEnvironmentChanged: () => void }) {
  const [backupBusy, setBackupBusy] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string }>({ ok: true, text: "" });
  const backup = async () => {
    setBackupBusy(true);
    try { setMessage({ ok: true, text: `配置备份已导出：${await api.backupConfig()}` }); }
    catch (e) { setMessage({ ok: false, text: String(e) }); }
    finally { setBackupBusy(false); }
  };
  return (
    <div className="view-scroll">
      <Stack gap="md">
        <div><Title order={3}>系统设置</Title><Text size="sm" c="dimmed">管理应用外观、更新、证书、备份与安全策略。</Text></div>
        {message.text && <Alert color={message.ok ? "teal" : "red"}>{message.text}</Alert>}
        <SimpleGrid cols={{ base: 1, md: 2 }}>
          <Card withBorder padding="lg" radius="lg"><Group mb="md"><ThemeIcon variant="light" size="lg"><IconPalette size={19} /></ThemeIcon><div><Text fw={700}>界面主题</Text><Text size="xs" c="dimmed">选择应用的强调色</Text></div></Group><SegmentedControl fullWidth value={scheme} onChange={(value) => setScheme(value as Scheme)} data={[{ value: "b", label: "深海蓝" }, { value: "a", label: "活力橙" }]} /></Card>
          <Card withBorder padding="lg" radius="lg"><Group justify="space-between" mb="md"><Group><ThemeIcon variant="light" size="lg"><IconDownload size={19} /></ThemeIcon><div><Text fw={700}>软件更新</Text><Text size="xs" c="dimmed">当前版本 v{appVersion || "--"}</Text></div></Group><Badge variant="light">稳定版</Badge></Group><Button variant="light" onClick={onCheckUpdate}>检查新版本</Button></Card>
          <Card withBorder padding="lg" radius="lg"><Group justify="space-between" mb="md"><Group><ThemeIcon variant="light" size="lg"><IconCertificate size={19} /></ThemeIcon><div><Text fw={700}>CA 证书</Text><Text size="xs" c="dimmed">公司网关的本地信任链</Text></div></Group></Group><CaCertButton env={env} onChanged={onEnvironmentChanged} /></Card>
          <Card withBorder padding="lg" radius="lg"><Group justify="space-between" mb="md"><Group><ThemeIcon variant="light" size="lg"><IconArchive size={19} /></ThemeIcon><div><Text fw={700}>配置备份</Text><Text size="xs" c="dimmed">导出不包含明文密钥的配置副本</Text></div></Group></Group><Button variant="light" onClick={backup} loading={backupBusy}>导出到桌面</Button></Card>
        </SimpleGrid>
        <Card withBorder padding="lg" radius="lg"><Group mb="md"><ThemeIcon color="teal" variant="light" size="lg"><IconShieldCheck size={19} /></ThemeIcon><div><Text fw={700}>安全策略</Text><Text size="xs" c="dimmed">当前应用遵循的本地安全边界</Text></div></Group><SimpleGrid cols={{ base: 1, sm: 3 }}><Alert color="teal" icon={<IconLock size={15} />}>API Key 使用系统安全存储，不进入诊断和配置备份。</Alert><Alert color="blue" icon={<IconShieldCheck size={15} />}>内容安全策略已启用，仅允许本地应用资源和 Tauri 通信。</Alert><Alert color="cyan" icon={<IconArchive size={15} />}>配置使用原子写入，并自动保留最近有效备份。</Alert></SimpleGrid></Card>
      </Stack>
    </div>
  );
}
