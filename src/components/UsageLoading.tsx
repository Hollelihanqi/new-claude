import { Button } from "@mantine/core";
import { IconChartLine, IconRefresh } from "@tabler/icons-react";

export default function UsageLoading({ error, onRetry }: { error?: string; onRetry?: () => void }) {
  return (
    <div className="usage-loading" role="status" aria-live="polite">
      <div className="usage-loading-mark" aria-hidden="true"><IconChartLine size={32} stroke={1.8} /></div>
      <h2>{error ? "洞察暂时无法加载" : "正在整理用量洞察"}</h2>
      <p>{error ? "读取本地记录时遇到问题，请重试。" : "正在读取本地会话记录，稍后就能看到完整的用量和趋势。"}</p>
      {error ? <>
        <span className="usage-loading-error">{error}</span>
        {onRetry && <Button variant="light" leftSection={<IconRefresh size={16} />} onClick={onRetry}>重新加载</Button>}
      </> : <>
        <div className="usage-loading-progress" aria-hidden="true"><span /></div>
        <div className="usage-loading-preview" aria-hidden="true"><i /><i /><i /><i /><i /><i /><i /></div>
      </>}
    </div>
  );
}
