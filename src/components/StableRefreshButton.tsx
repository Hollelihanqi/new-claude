import { Button, Loader } from "@mantine/core";
import type { ButtonProps } from "@mantine/core";
import { IconRefresh } from "@tabler/icons-react";
import { useCallback, useEffect, useRef, useState } from "react";

export const MINIMUM_REFRESH_FEEDBACK_MS = 650;

export default function StableRefreshButton({
  busy,
  busyLabel = "刷新中…",
  color,
  disabled,
  iconSize = 15,
  label,
  onClick,
  size,
}: {
  busy: boolean;
  busyLabel?: string;
  color?: ButtonProps["color"];
  disabled?: boolean;
  iconSize?: number;
  label: string;
  onClick: () => void;
  size?: ButtonProps["size"];
}) {
  const [minimumBusy, setMinimumBusy] = useState(false);
  const minimumBusyRef = useRef(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const startMinimumFeedback = useCallback(() => {
    if (minimumBusyRef.current) return;

    minimumBusyRef.current = true;
    setMinimumBusy(true);
    timerRef.current = setTimeout(() => {
      minimumBusyRef.current = false;
      timerRef.current = null;
      setMinimumBusy(false);
    }, MINIMUM_REFRESH_FEEDBACK_MS);
  }, []);

  useEffect(() => {
    if (busy) startMinimumFeedback();
  }, [busy, startMinimumFeedback]);

  useEffect(() => () => {
    if (timerRef.current) clearTimeout(timerRef.current);
  }, []);

  const visibleBusy = busy || minimumBusy;

  return (
    <Button
      size={size}
      variant="light"
      color={color}
      disabled={disabled}
      leftSection={(
        visibleBusy
          ? <Loader size={iconSize} className="stable-refresh-loader" />
          : <IconRefresh size={iconSize} className="stable-refresh-icon" />
      )}
      className="stable-refresh-button"
      data-busy={visibleBusy || undefined}
      aria-busy={visibleBusy}
      aria-disabled={visibleBusy}
      onClick={() => {
        if (visibleBusy) return;
        startMinimumFeedback();
        onClick();
      }}
    >
      <span className="stable-refresh-label-stack">
        <span className="stable-refresh-label idle" aria-hidden={visibleBusy}>
          {label}
        </span>
        <span className="stable-refresh-label busy" aria-hidden={!visibleBusy}>
          {busyLabel}
        </span>
      </span>
    </Button>
  );
}
