import { useMemo, useState, type CSSProperties } from "react";
import { Alert, Badge, Button, Stack, Text } from "@mantine/core";
import {
  IconArchive,
  IconCertificate,
  IconCheck,
  IconChevronRight,
  IconDownload,
  IconInfoCircle,
  IconLock,
  IconPalette,
  IconShieldCheck,
  IconSparkles,
} from "@tabler/icons-react";
import { api } from "../api";
import type { EnvInfo } from "../api";
import {
  THEME_DEFINITIONS,
  type Scheme,
  type ThemeCategory,
  type ThemeDefinition,
} from "../themeScheme";
import oceanWave from "../assets/themes/ocean-wave.png";
import appleGlass from "../assets/themes/apple-glass.png";
import springArt from "../assets/themes/festival-spring.png";
import lanternArt from "../assets/themes/festival-lantern.png";
import dragonBoatArt from "../assets/themes/festival-dragonboat.png";
import midAutumnArt from "../assets/themes/festival-midautumn.png";
import nationalArt from "../assets/themes/festival-national.png";
import CaCertButton from "./CaCertButton";

type ThemeStyle = CSSProperties & Record<`--${string}`, string>;

const THEME_ART: Partial<Record<Scheme, string>> = {
  apple: appleGlass,
  spring: springArt,
  lantern: lanternArt,
  dragonboat: dragonBoatArt,
  midautumn: midAutumnArt,
  national: nationalArt,
};

function previewStyle(theme: ThemeDefinition): ThemeStyle {
  return {
    "--preview-base": theme.colors[0],
    "--preview-surface": theme.colors[1],
    "--preview-accent": theme.colors[2],
    "--preview-soft": theme.colors[3],
    "--preview-risk": theme.colors[4],
  };
}

function ThemeMiniature({ theme }: { theme: ThemeDefinition }) {
  const art = THEME_ART[theme.value];
  return (
    <div
      className="theme-miniature"
      style={previewStyle(theme)}
      data-festival={theme.category === "festival"}
      data-apple={theme.value === "apple"}
      aria-hidden="true"
    >
      {art && <img src={art} alt="" className="theme-mini-art" />}
      <div className="theme-mini-sidebar">
        <span className="theme-mini-dot" />
        <span />
        <span />
        <span />
      </div>
      <div className="theme-mini-canvas">
        <span className="theme-mini-heading" />
        <span />
        <span />
      </div>
      <div className="theme-mini-chart">
        <i /><i /><i /><i />
      </div>
    </div>
  );
}

function ThemeCard({
  theme,
  active,
  onSelect,
}: {
  theme: ThemeDefinition;
  active: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      className={`theme-choice ${active ? "active" : ""}`}
      aria-pressed={active}
      onClick={onSelect}
    >
      <ThemeMiniature theme={theme} />
      <span className="theme-choice-meta">
        <span className="theme-choice-title">
          <i style={{ background: theme.colors[2] }} />
          <strong>{theme.name}</strong>
          {active && <IconCheck size={16} stroke={2.4} />}
        </span>
        <small>{theme.description}</small>
      </span>
    </button>
  );
}

function LiveThemePreview({ theme }: { theme: ThemeDefinition }) {
  const art = THEME_ART[theme.value] ?? oceanWave;
  return (
    <section
      className="theme-live-preview"
      style={previewStyle(theme)}
      data-festival={theme.category === "festival"}
      data-apple={theme.value === "apple"}
    >
      <img src={art} alt="" className="theme-live-art" />
      <div className="theme-live-copy">
        <Text className="theme-live-name">{theme.name}</Text>
        <Text className="theme-live-description">{theme.description}</Text>
        <Badge variant="light" color="gray">已应用当前主题</Badge>
      </div>
      <div className="theme-live-window" aria-hidden="true">
        <div className="theme-live-window-sidebar">
          <span className="theme-live-brand" />
          <i /><i className="active" /><i />
        </div>
        <div className="theme-live-window-content">
          <b />
          <span />
          <div className="theme-live-chart"><i /><i /><i /><i /></div>
        </div>
      </div>
    </section>
  );
}

export default function SettingsPanel({
  env,
  scheme,
  setScheme,
  appVersion,
  onCheckUpdate,
  onEnvironmentChanged,
}: {
  env: EnvInfo | null;
  scheme: Scheme;
  setScheme: (value: Scheme) => void;
  appVersion: string;
  onCheckUpdate: () => void;
  onEnvironmentChanged: () => void;
}) {
  const selected = THEME_DEFINITIONS.find((theme) => theme.value === scheme) ?? THEME_DEFINITIONS[0];
  const [category, setCategory] = useState<ThemeCategory>(selected.category);
  const [backupBusy, setBackupBusy] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string }>({ ok: true, text: "" });
  const visibleThemes = useMemo(
    () => THEME_DEFINITIONS.filter((theme) => theme.category === category),
    [category]
  );
  const dailyCount = THEME_DEFINITIONS.filter((theme) => theme.category === "daily").length;
  const festivalCount = THEME_DEFINITIONS.filter((theme) => theme.category === "festival").length;

  const backup = async () => {
    setBackupBusy(true);
    try {
      setMessage({ ok: true, text: `配置备份已导出：${await api.backupConfig()}` });
    } catch (error) {
      setMessage({ ok: false, text: String(error) });
    } finally {
      setBackupBusy(false);
    }
  };

  const chooseTheme = (theme: ThemeDefinition) => setScheme(theme.value);

  return (
    <div className="view-scroll settings-page">
      <Stack gap="md">
        {message.text && (
          <Alert
            className={message.ok ? "semantic-success" : "semantic-danger"}
            color={message.ok ? "teal" : "red"}
            icon={message.ok ? <IconCheck size={16} /> : undefined}
          >
            {message.text}
          </Alert>
        )}

        <section className="theme-studio">
          <header className="theme-studio-header">
            <span className="settings-section-icon"><IconPalette size={21} /></span>
            <div>
              <Text fw={720} size="lg">界面主题</Text>
              <Text size="sm" c="dimmed">选择完整视觉风格，背景、导航、控件与数据配色会同步更新。</Text>
            </div>
          </header>

          <div className="theme-category-bar" role="tablist" aria-label="主题分类">
            <button
              role="tab"
              aria-selected={category === "daily"}
              className={category === "daily" ? "active" : ""}
              onClick={() => setCategory("daily")}
            >
              日常风格 <span>{dailyCount}</span>
            </button>
            <button
              role="tab"
              aria-selected={category === "festival"}
              className={category === "festival" ? "active" : ""}
              onClick={() => setCategory("festival")}
            >
              节日限定 <span>{festivalCount}</span>
            </button>
          </div>

          <div className={`theme-gallery theme-gallery-${category}`} role="tabpanel">
            {visibleThemes.map((theme) => (
              <ThemeCard
                key={theme.value}
                theme={theme}
                active={scheme === theme.value}
                onSelect={() => chooseTheme(theme)}
              />
            ))}
          </div>

          <LiveThemePreview theme={selected} />
        </section>

        <section className="settings-list" aria-label="其他设置">
          <header className="settings-list-title">
            <Text fw={700}>其他设置</Text>
            <Text size="xs" c="dimmed">更新、证书与本地数据</Text>
          </header>

          <div className="settings-row">
            <span className="settings-row-icon"><IconDownload size={19} /></span>
            <div className="settings-row-copy">
              <strong>软件更新</strong>
              <small>当前版本 v{appVersion || "--"}</small>
            </div>
            <Badge variant="light" color="gray">稳定版</Badge>
            <Button variant="subtle" rightSection={<IconChevronRight size={15} />} onClick={onCheckUpdate}>检查新版本</Button>
          </div>

          <div className="settings-row">
            <span className="settings-row-icon"><IconCertificate size={19} /></span>
            <div className="settings-row-copy">
              <strong>CA 证书</strong>
              <small>公司网关的本地信任链</small>
            </div>
            <CaCertButton env={env} onChanged={onEnvironmentChanged} />
          </div>

          <div className="settings-row">
            <span className="settings-row-icon"><IconArchive size={19} /></span>
            <div className="settings-row-copy">
              <strong>配置备份</strong>
              <small>导出到桌面；Windows 上这份副本包含密钥密文</small>
            </div>
            <Button
              variant="subtle"
              rightSection={<IconChevronRight size={15} />}
              onClick={backup}
              loading={backupBusy}
            >
              导出到桌面
            </Button>
          </div>
        </section>

        <section className="settings-assurance" aria-label="安全说明">
          <div className="settings-assurance-item semantic-info-row">
            <IconInfoCircle size={17} />
            <span>传统节日当天会自动启用对应皮肤；你仍可随时手动切换，无需重启应用。</span>
          </div>
          <div className="settings-assurance-item">
            <IconLock size={17} />
            <span>网关 Key 由系统安全存储保护；WorkBuddy 与 MCP 密钥仍是本地明文文件。</span>
          </div>
          <div className="settings-assurance-item">
            <IconShieldCheck size={17} />
            <span>内容安全策略、原子写入和最近有效备份持续启用。</span>
          </div>
          <div className="settings-assurance-item settings-assurance-note">
            <IconSparkles size={17} />
            <span>节日皮肤只改变视觉表达，不会覆盖成功、提醒和风险的语义颜色。</span>
          </div>
        </section>
      </Stack>
    </div>
  );
}
