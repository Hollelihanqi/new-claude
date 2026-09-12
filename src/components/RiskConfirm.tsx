import { Alert, Button, Checkbox, Group, Modal, Stack, Text } from "@mantine/core";
import { IconAlertTriangle, IconShieldLock } from "@tabler/icons-react";
import { useState, type ReactNode } from "react";

/**
 * 危险操作风险分级（P0-B#8）。
 *
 * 判据是「**能不能撤销**」+「**是否改变安全边界**」，不是"感觉危不危险"：
 *
 * | 级别 | 判据 | 交互要求 |
 * |---|---|---|
 * | 常规 | 可逆，只影响界面偏好 | 直接执行，**不用**本组件 |
 * | `high` | **不可逆**（数据丢失，或需要重新配置才能恢复） | 弹窗 + 危险色按钮 + **逐条列出具体后果** |
 * | `critical` | **改变安全边界**（系统级信任、权限策略） | `high` 的全部，**外加必须勾选「我已了解影响范围」** |
 *
 * 两条刻意的设计：
 *
 * 1. **`consequences` 必填且逐条渲染**。写不出具体后果，说明这个动作要么不该做、
 *    要么分级定错了。泛泛的「确定吗？」不构成确认 —— 它只训练用户闭眼点确认。
 * 2. **`critical` 必须显式勾选**，且勾选状态在每次打开时重置，避免残留导致
 *    下一次误触直接放行。
 *
 * 现实中的反例（本组件就是为修它而建）：`bypassPermissions` 曾是一个**没有任何确认**
 * 的开关，一开就持久关闭该环境所有权限询问；而"删除一个 WorkBuddy 组织"却要弹窗 ——
 * 风险与摩擦完全不成比例。
 */
export type RiskLevel = "high" | "critical";

interface Props {
  opened: boolean;
  level: RiskLevel;
  title: string;
  /** 逐条列出**具体**后果。空数组在开发期会报错（见文件头说明）。 */
  consequences: string[];
  /** 可选：操作对象的细节，如要导入的证书文件路径 —— 涉及信任变更时应让用户看清作用对象 */
  detail?: ReactNode;
  confirmLabel: string;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export default function RiskConfirm({
  opened,
  level,
  title,
  consequences,
  detail,
  confirmLabel,
  busy,
  onConfirm,
  onCancel,
}: Props) {
  const [acknowledged, setAcknowledged] = useState(false);
  const critical = level === "critical";

  if (consequences.length === 0 && import.meta.env.DEV) {
    console.error(`[RiskConfirm] 「${title}」没有列出具体后果，这不构成有效确认。`);
  }

  // 每次关闭都重置勾选：否则上次勾过、这次一开就直接可点确认。
  //
  // 光在 close() 里重置**不够** —— 父组件可能直接把 opened 置 false（不走 onCancel），
  // 此时 close() 不会被调用，勾选会残留到下次打开，一次点击就能确认。
  // 所以同时监听 opened 由 true→false 的变化（React 官方的"渲染期调整派生状态"写法）。
  const [prevOpened, setPrevOpened] = useState(opened);
  if (opened !== prevOpened) {
    setPrevOpened(opened);
    if (!opened && acknowledged) setAcknowledged(false);
  }

  const close = () => {
    setAcknowledged(false);
    onCancel();
  };
  const confirm = () => {
    if (critical && !acknowledged) return;
    setAcknowledged(false);
    onConfirm();
  };

  return (
    <Modal opened={opened} onClose={close} title={title} centered>
      <Stack gap="sm">
        <Alert
          color={critical ? "red" : "orange"}
          icon={critical ? <IconShieldLock size={16} /> : <IconAlertTriangle size={16} />}
        >
          <Stack gap={4}>
            {consequences.map((line, index) => (
              <Text key={index} size="sm">
                · {line}
              </Text>
            ))}
          </Stack>
        </Alert>

        {detail && (
          <Text size="xs" c="dimmed" style={{ wordBreak: "break-all" }}>
            {detail}
          </Text>
        )}

        {critical && (
          <Checkbox
            checked={acknowledged}
            onChange={(event) => setAcknowledged(event.currentTarget.checked)}
            label="我已了解上述影响范围"
          />
        )}

        <Group justify="flex-end">
          <Button variant="default" onClick={close}>
            取消
          </Button>
          <Button
            color="red"
            loading={busy}
            disabled={critical && !acknowledged}
            onClick={confirm}
          >
            {confirmLabel}
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}
