import { useState } from "react";
import {
  Button,
  Modal,
  Stack,
  TextInput,
  Group,
  Text,
  Badge,
  Alert,
} from "@mantine/core";
import { IconCertificate, IconInfoCircle } from "@tabler/icons-react";
import { api } from "../api";
import type { EnvInfo } from "../api";

type StatusType = "info" | "error" | "success";

// 整机共享的 CA 证书管理：header 里一个按钮，点开弹框做导入 / 清空。
// 证书是全局的，跟具体实例无关，所以放在顶栏而非配置页左栏。
export default function CaCertButton({
  env,
  onChanged,
}: {
  env: EnvInfo | null;
  onChanged?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [certPath, setCertPath] = useState("");
  const [msg, setMsg] = useState<{ type: StatusType; msg: string }>({
    type: "info",
    msg: "",
  });
  const [busy, setBusy] = useState("");

  const certCount = env?.cert_count ?? 0;
  const certPlaceholder =
    env?.platform === "windows" ? "C:\\ca-cert.pem" : "/Users/you/ca-cert.pem";

  const onImport = async () => {
    if (!certPath.trim()) {
      setMsg({ type: "error", msg: "请填写证书文件（ca-cert.pem）的完整路径。" });
      return;
    }
    setBusy("import");
    try {
      const m = await api.importCert(certPath.trim());
      onChanged && onChanged();
      setCertPath("");
      setMsg({ type: "success", msg: m });
    } catch (e) {
      setMsg({ type: "error", msg: String(e) });
    } finally {
      setBusy("");
    }
  };

  const onClear = async () => {
    setBusy("clear");
    try {
      const m = await api.clearCerts();
      onChanged && onChanged();
      setMsg({ type: "success", msg: m });
    } catch (e) {
      setMsg({ type: "error", msg: String(e) });
    } finally {
      setBusy("");
    }
  };

  return (
    <>
      <Button
        size="xs"
        variant="light"
        leftSection={<IconCertificate size={14} />}
        onClick={() => setOpen(true)}
      >
        <Group gap={6} wrap="nowrap">
          <span>CA 证书</span>
          <Badge
            size="xs"
            variant="filled"
            color={certCount ? "teal" : "gray"}
            styles={{ root: { textTransform: "none" } }}
          >
            {certCount ? `${certCount}` : "0"}
          </Badge>
        </Group>
      </Button>

      <Modal
        opened={open}
        onClose={() => setOpen(false)}
        title={
          <Group gap={6} wrap="nowrap">
            <IconCertificate size={18} />
            <Text fw={600}>CA 证书</Text>
            <Badge size="sm" variant="light" color={certCount ? "teal" : "gray"}>
              {certCount ? `${certCount} 张` : "未导入"}
            </Badge>
          </Group>
        }
        size="md"
        centered
      >
        <Stack gap="sm">
          <Text size="sm" c="dimmed">
            整机共享：多张证书会自动合并到一个信任库，所有实例通用。
          </Text>

          {msg.msg && (
            <Alert
              variant="light"
              color={
                msg.type === "error"
                  ? "red"
                  : msg.type === "success"
                  ? "teal"
                  : "blue"
              }
              icon={<IconInfoCircle size={16} />}
            >
              {msg.msg}
            </Alert>
          )}

          <TextInput
            label="证书文件路径"
            description="ca-cert.pem 的完整路径，导入多张会自动合并。"
            placeholder={certPlaceholder}
            value={certPath}
            onChange={(e) => setCertPath(e.currentTarget.value)}
          />

          <Group gap="xs">
            <Button size="sm" onClick={onImport} loading={busy === "import"}>
              导入
            </Button>
            <Button
              size="sm"
              variant="light"
              color="red"
              onClick={onClear}
              loading={busy === "clear"}
              disabled={!certCount}
            >
              清空
            </Button>
          </Group>

          <Text size="xs" c="dimmed">
            清空会移除全部已导入证书，操作不可撤销。
          </Text>
        </Stack>
      </Modal>
    </>
  );
}
