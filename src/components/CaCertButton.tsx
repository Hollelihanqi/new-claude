import { usePageActive } from "./PersistentPage";
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
  Select,
} from "@mantine/core";
import { IconCertificate, IconInfoCircle } from "@tabler/icons-react";
import { api } from "../api";
import type { EnvInfo } from "../api";
import RiskConfirm from "./RiskConfirm";

type StatusType = "info" | "error" | "success";

/** 「所有网关环境」的哨兵值：它是一次**循环导入**，不是共享文件。 */
const ALL_TARGET = "__all__";

// CA 证书管理：header 里一个按钮，点开弹框做导入 / 清空。
//
// **CA 是每个网关各自一份**（约束 5：网关之间的 CA 彼此隔离）。
// 给网关 A 导入的私有 CA 不会让网关 B 也信任它 —— 否则 B 的中间人信任面被无谓放大。
// 需要多个网关共用同一张 CA 时，选「所有网关环境」——那是对每个网关各写一次，
// 之后仍可逐个移除。
export default function CaCertButton({
  env,
  onChanged,
}: {
  env: EnvInfo | null;
  onChanged?: () => void;
}) {
  const pageActive = usePageActive();
  const [open, setOpen] = useState(false);
  const [certPath, setCertPath] = useState("");
  const [msg, setMsg] = useState<{ type: StatusType; msg: string }>({
    type: "info",
    msg: "",
  });
  const [busy, setBusy] = useState("");
  // 清空证书不可逆（自签网关会立刻连不上），走 P0-B#8 的 high 级确认
  const [clearOpen, setClearOpen] = useState(false);
  // 导入 CA 会扩大**目标网关**的信任范围，同样是需要确认的信任边界变更
  const [importOpen, setImportOpen] = useState(false);
  // 目标网关：默认「所有网关环境」（与改造前行为一致），也可以只作用于某一个
  const [target, setTarget] = useState<string>(ALL_TARGET);
  const [gateways, setGateways] = useState<string[]>([]);

  const certCount = env?.cert_count ?? 0;
  const certPlaceholder =
    env?.platform === "windows" ? "C:\\ca-cert.pem" : "/Users/you/ca-cert.pem";

  // 只在**网关环境**里选目标：独立登录环境不注入 CA，列出来只会误导。
  const loadGateways = () => {
    api
      .listProfiles()
      .then((ps) => setGateways(ps.filter((p) => p.type === "router").map((p) => p.name)))
      .catch(() => {});
  };

  const targets = target === ALL_TARGET ? gateways : [target];
  const targetLabel =
    target === ALL_TARGET ? `所有网关环境（${gateways.length} 个）` : `网关 ${target}`;

  // 返回**是否成功**。原先这两个 helper 把错误吞进 msg 后正常 resolve，
  // 调用方无从判断成败，于是失败时仍然关闭确认框 / 清空已填路径，丢掉重试上下文。
  const onImport = async (): Promise<boolean> => {
    if (!certPath.trim()) {
      setMsg({ type: "error", msg: "请填写证书文件（ca-cert.pem）的完整路径。" });
      return false;
    }
    if (targets.length === 0) {
      setMsg({ type: "error", msg: "还没有网关环境，无法导入 CA。" });
      return false;
    }
    setBusy("import");
    try {
      const m = await api.importCertFor(targets, certPath.trim());
      onChanged && onChanged();
      setCertPath("");
      setMsg({ type: "success", msg: m });
      return true;
    } catch (e) {
      setMsg({ type: "error", msg: String(e) });
      return false;
    } finally {
      setBusy("");
    }
  };

  const onClear = async (): Promise<boolean> => {
    if (targets.length === 0) {
      setMsg({ type: "error", msg: "还没有网关环境，没有可清空的 CA。" });
      return false;
    }
    setBusy("clear");
    try {
      const m = await api.clearCertsFor(targets);
      onChanged && onChanged();
      setMsg({ type: "success", msg: m });
      return true;
    } catch (e) {
      setMsg({ type: "error", msg: String(e) });
      return false;
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
        onClick={() => {
          loadGateways();
          setOpen(true);
        }}
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
        opened={pageActive && (open)}
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
            CA 按网关「各自保存」：给某个网关导入的证书只作用于它自己，
            不会让其他网关也信任这张 CA。需要多个网关共用时选「所有网关环境」——
            那是逐个写入，之后仍可单独移除。
          </Text>

          <Select
            label="作用范围"
            data={[
              {
                value: ALL_TARGET,
                label: `所有网关环境（${gateways.length} 个）`,
              },
              ...gateways.map((g) => ({ value: g, label: `网关 ${g}` })),
            ]}
            value={target}
            onChange={(v) => v && setTarget(v)}
            allowDeselect={false}
          />

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
              {/* 清理结果是逐条的多行汇报（哪一步成功、哪一步失败），必须保留换行 */}
              <Text size="sm" style={{ whiteSpace: "pre-line" }}>
                {msg.msg}
              </Text>
            </Alert>
          )}

          <TextInput
            label="证书文件路径"
            description="ca-cert.pem 的完整路径；多张证书会合并进所选范围，不会外溢到别的网关。"
            placeholder={certPlaceholder}
            value={certPath}
            onChange={(e) => setCertPath(e.currentTarget.value)}
          />

          <Group gap="xs">
            <Button
              size="sm"
              onClick={() => setImportOpen(true)}
              disabled={!certPath.trim() || gateways.length === 0}
            >
              导入
            </Button>
            <Button
              size="sm"
              variant="light"
              color="red"
              onClick={() => setClearOpen(true)}
              disabled={gateways.length === 0}
            >
              清空
            </Button>
          </Group>

          <Text size="xs" c="dimmed">
            以上操作作用于「{targetLabel}」；清空仅移除该范围的证书，
            仍被其他网关注的证书不会被撤销出系统信任库。
          </Text>
        </Stack>

        <RiskConfirm
          opened={pageActive && clearOpen}
          level="high"
          title="清空所选范围的 CA 证书"
          consequences={[
            `将移除 ${targetLabel} 的已导入证书，无法撤销。`,
            "依赖这些证书的自签网关会立刻连不上，必须重新导入证书才能恢复。",
            "仍被其他网关注用的证书不会被撤销出系统信任库 —— 只清掉这些网关自己的那份。",
          ]}
          confirmLabel="确认清空"
          busy={busy === "clear"}
          onCancel={() => setClearOpen(false)}
          onConfirm={async () => {
            // 失败时保持打开，保留上下文让用户重试
            if (await onClear()) setClearOpen(false);
          }}
        />

        {/* 导入同样是信任边界变更：证书会成为**全部托管环境**的信任根。
            原先进口是直接执行、没有任何确认 —— 只挡了"清空"却放过了"扩大信任"。 */}
        <RiskConfirm
          opened={pageActive && importOpen}
          level="critical"
          title="导入并信任 CA 证书"
          consequences={[
            `该 CA 会成为「${targetLabel}」的信任根：此后本应用启动的这些 claude 会话都信任它签发的任意证书。`,
            "导错证书的代价是：那个 CA 能对这些网关做中间人，读到 API Key 与全部请求内容。",
            "不会波及其他网关 —— CA 与网关是一对一的，选「所有网关环境」才会逐个写入。",
            "只影响本应用启动的 claude 会话，不会改动系统信任库，也不影响浏览器等其它程序。",
            "请只导入网关管理员提供的、来源可核对的根证书。",
          ]}
          detail={certPath.trim()}
          confirmLabel="确认导入并信任"
          busy={busy === "import"}
          onCancel={() => setImportOpen(false)}
          onConfirm={async () => {
            // 失败时保持打开且**不清空已填路径**，用户可直接重试或改路径
            if (await onImport()) setImportOpen(false);
          }}
        />
      </Modal>
    </>
  );
}
