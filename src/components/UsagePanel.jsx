import { useEffect, useMemo, useRef, useState } from "react";
import {
  Stack,
  Group,
  Card,
  Text,
  Title,
  SegmentedControl,
  SimpleGrid,
  Button,
  Alert,
  Badge,
} from "@mantine/core";
import { IconRefresh, IconChartLine, IconInfoCircle } from "@tabler/icons-react";
import * as echarts from "echarts";
import { api } from "../api.js";

// 通用 echarts 容器
function EChart({ option, height = 320 }) {
  const ref = useRef(null);
  const inst = useRef(null);
  useEffect(() => {
    if (!ref.current) return;
    inst.current = echarts.init(ref.current);
    const onResize = () => inst.current && inst.current.resize();
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      inst.current && inst.current.dispose();
    };
  }, []);
  useEffect(() => {
    if (inst.current) inst.current.setOption(option, true);
  }, [option]);
  return <div ref={ref} style={{ width: "100%", height }} />;
}

const fmt = (n) => {
  if (n >= 1e9) return (n / 1e9).toFixed(2) + "B";
  if (n >= 1e6) return (n / 1e6).toFixed(2) + "M";
  if (n >= 1e3) return (n / 1e3).toFixed(1) + "K";
  return String(n);
};

// 取某天所在周的周一（YYYY-MM-DD）
function weekStart(dateStr) {
  const d = new Date(dateStr + "T00:00:00Z");
  const day = (d.getUTCDay() + 6) % 7; // 周一=0
  d.setUTCDate(d.getUTCDate() - day);
  return d.toISOString().slice(0, 10);
}

function StatCard({ label, value, sub, color }) {
  return (
    <Card withBorder padding="md" radius="md">
      <Text size="xs" c="dimmed">
        {label}
      </Text>
      <Text fw={700} size="xl" c={color}>
        {value}
      </Text>
      {sub && (
        <Text size="xs" c="dimmed">
          {sub}
        </Text>
      )}
    </Card>
  );
}

export default function UsagePanel() {
  const [data, setData] = useState(null);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [gran, setGran] = useState("day");

  const load = () => {
    setBusy(true);
    setErr("");
    api
      .usageStats()
      .then(setData)
      .catch((e) => setErr(String(e)))
      .finally(() => setBusy(false));
  };
  useEffect(load, []);

  const rows = data?.daily || [];

  // 按粒度聚合成时间序列
  const series = useMemo(() => {
    const bucket = new Map();
    for (const r of rows) {
      let key = r.date;
      if (gran === "week") key = weekStart(r.date);
      else if (gran === "month") key = r.date.slice(0, 7);
      const cur = bucket.get(key) || { input: 0, output: 0, cacheRead: 0 };
      cur.input += r.input;
      cur.output += r.output;
      cur.cacheRead += r.cacheRead;
      bucket.set(key, cur);
    }
    const labels = Array.from(bucket.keys()).sort();
    return {
      labels,
      input: labels.map((l) => bucket.get(l).input),
      output: labels.map((l) => bucket.get(l).output),
      cacheRead: labels.map((l) => bucket.get(l).cacheRead),
    };
  }, [rows, gran]);

  // 按模型汇总
  const byModel = useMemo(() => {
    const m = new Map();
    for (const r of rows) m.set(r.model, (m.get(r.model) || 0) + r.input + r.output);
    return Array.from(m.entries())
      .map(([name, value]) => ({ name, value }))
      .sort((a, b) => b.value - a.value);
  }, [rows]);

  // 按实例汇总
  const byProfile = useMemo(() => {
    const m = new Map();
    for (const r of rows) m.set(r.profile, (m.get(r.profile) || 0) + r.input + r.output);
    return Array.from(m.entries())
      .map(([name, value]) => ({ name, value }))
      .sort((a, b) => b.value - a.value);
  }, [rows]);

  const lineOption = useMemo(
    () => ({
      tooltip: { trigger: "axis" },
      legend: { data: ["输入", "输出", "缓存读取"], top: 0 },
      grid: { left: 50, right: 16, top: 36, bottom: 40 },
      xAxis: { type: "category", data: series.labels, axisLabel: { rotate: 30 } },
      yAxis: { type: "value", axisLabel: { formatter: (v) => fmt(v) } },
      series: [
        {
          name: "输入",
          type: "line",
          smooth: true,
          data: series.input,
          areaStyle: { opacity: 0.12 },
          itemStyle: { color: "#4c6ef5" },
        },
        {
          name: "输出",
          type: "line",
          smooth: true,
          data: series.output,
          areaStyle: { opacity: 0.12 },
          itemStyle: { color: "#37b24d" },
        },
        {
          name: "缓存读取",
          type: "line",
          smooth: true,
          data: series.cacheRead,
          itemStyle: { color: "#f59f00" },
        },
      ],
    }),
    [series]
  );

  const pieOption = useMemo(
    () => ({
      tooltip: { trigger: "item", formatter: (p) => `${p.name}<br/>${fmt(p.value)} (${p.percent}%)` },
      legend: { type: "scroll", bottom: 0 },
      series: [
        {
          type: "pie",
          radius: ["40%", "70%"],
          data: byModel,
          label: { formatter: "{b}" },
        },
      ],
    }),
    [byModel]
  );

  const barOption = useMemo(
    () => ({
      tooltip: { trigger: "axis", formatter: (a) => `${a[0].name}<br/>${fmt(a[0].value)}` },
      grid: { left: 60, right: 16, top: 16, bottom: 30 },
      xAxis: { type: "category", data: byProfile.map((d) => d.name) },
      yAxis: { type: "value", axisLabel: { formatter: (v) => fmt(v) } },
      series: [
        {
          type: "bar",
          data: byProfile.map((d) => d.value),
          itemStyle: { color: "#4c6ef5" },
          barMaxWidth: 48,
        },
      ],
    }),
    [byProfile]
  );

  const hasData = rows.length > 0;

  return (
    <Stack gap="md">
      <Group justify="space-between">
        <Group gap="xs">
          <Title order={5}>用量统计</Title>
          <Badge variant="light" color="gray">
            本地会话记录
          </Badge>
        </Group>
        <Button
          size="xs"
          variant="light"
          leftSection={<IconRefresh size={14} />}
          onClick={load}
          loading={busy}
        >
          刷新
        </Button>
      </Group>

      {err && (
        <Alert color="red" icon={<IconInfoCircle size={16} />}>
          {err}
        </Alert>
      )}

      <Alert variant="light" color="blue" icon={<IconInfoCircle size={16} />}>
        数据来自本机 Claude Code 的会话记录（各实例的对话日志），统计你在这台电脑上的
        token 用量。失败/未连通的请求不计入。这不是网关的官方账单，但能反映本机实际使用。
      </Alert>

      {!hasData && !busy && (
        <Card withBorder padding="xl" radius="md">
          <Stack align="center" gap="xs">
            <IconChartLine size={40} opacity={0.4} />
            <Text c="dimmed" size="sm">
              还没有可统计的用量。用 claude 跑几次对话后回来刷新即可。
            </Text>
          </Stack>
        </Card>
      )}

      {hasData && (
        <>
          <SimpleGrid cols={{ base: 2, sm: 4 }}>
            <StatCard
              label="总输入 token"
              value={fmt(data.totalInput)}
              color="indigo"
            />
            <StatCard
              label="总输出 token"
              value={fmt(data.totalOutput)}
              color="teal"
            />
            <StatCard label="请求次数" value={fmt(data.totalRequests)} />
            <StatCard
              label="输入+输出合计"
              value={fmt(data.totalInput + data.totalOutput)}
            />
          </SimpleGrid>

          <Card withBorder padding="md" radius="md">
            <Group justify="space-between" mb="xs">
              <Text fw={600}>使用曲线</Text>
              <SegmentedControl
                size="xs"
                value={gran}
                onChange={setGran}
                data={[
                  { label: "天", value: "day" },
                  { label: "周", value: "week" },
                  { label: "月", value: "month" },
                ]}
              />
            </Group>
            <EChart option={lineOption} height={340} />
          </Card>

          <SimpleGrid cols={{ base: 1, md: 2 }}>
            <Card withBorder padding="md" radius="md">
              <Text fw={600} mb="xs">
                各模型用量占比
              </Text>
              <EChart option={pieOption} height={300} />
            </Card>
            <Card withBorder padding="md" radius="md">
              <Text fw={600} mb="xs">
                各实例用量对比
              </Text>
              <EChart option={barOption} height={300} />
            </Card>
          </SimpleGrid>
        </>
      )}
    </Stack>
  );
}
