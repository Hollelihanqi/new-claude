import type { ReactNode } from "react";
import { Button, Modal, Stack } from "@mantine/core";
import { IconAlertTriangle, IconBell, IconCircleCheck, IconInfoCircle } from "@tabler/icons-react";

export type StatusModalVariant = "danger" | "success" | "info" | "primary";

const APPEARANCE = {
  danger: { icon: IconAlertTriangle, color: "red" },
  success: { icon: IconCircleCheck, color: "teal" },
  info: { icon: IconInfoCircle, color: "blue" },
  primary: { icon: IconBell, color: undefined },
} as const;

interface Props {
  opened: boolean;
  onClose: () => void;
  title: string;
  variant?: StatusModalVariant;
  subject?: string;
  description: ReactNode;
  dismissLabel?: string;
}

export default function StatusModal({ opened, onClose, title, variant = "info", subject, description, dismissLabel = "知道了" }: Props) {
  const { icon: Icon, color } = APPEARANCE[variant];
  return <Modal opened={opened} onClose={onClose} centered withCloseButton={false}
    title={<span className="status-modal-heading"><span className="status-modal-icon" aria-hidden="true"><Icon size={23} stroke={1.8} /></span><span>{title}</span></span>}
    classNames={{ content: `status-modal-content status-modal-${variant}`, header: "status-modal-header", title: "status-modal-title", body: "status-modal-body" }}>
    <Stack gap="md" align="center">
      {subject && <span className="status-modal-subject">{subject}</span>}
      <div className="status-modal-description">{description}</div>
      <Button className="status-modal-dismiss" color={color} onClick={onClose}>{dismissLabel}</Button>
    </Stack>
  </Modal>;
}
