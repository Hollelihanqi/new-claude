import { useEffect, useMemo, useRef, useState } from "react";
import {
  Stack,
  Group,
  Card,
  Text,
  Title,
  SimpleGrid,
  Button,
  Alert,
  Badge,
  Select,
} from "@mantine/core";
import { IconRefresh, IconChartLine, IconInfoCircle } from "@tabler/icons-react";
import * as echarts from "echarts";
import { api } from "../api.js";

function EChart({ option, height = 340 }) {
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
  return String(Math.round(n));
};

const RANGE_OPTS = [
  { value: "today", label: "当天（按小时）" },
  { value: "7", label: "近 7 天" },
  { value: "14", label: "近 14 天" },
  { value: "30", label: "近 30 天" },
  { value: "all", label: "全部" },
];

// 主题线条配色
const lineColors = (scheme) =>
  scheme === "a"
    ? { input: "#fd752c", output: "#185a56", cacheCreate: "#e8950c", cacheRead: "#9aa0a6" }
    : { input: "#0c7e9e", output: "#b89878", cacheCreate: "#e8950c", cacheRead: "#9aa0a6" };

function StatCard({ label, value, color }) {
  return (
    <Card withBorder padding="md" radius="lg">
      <Text size="xs" c="dimmed">
        {label}
      </Text>
      <Text fw={700} size="xl" c={color}>
        {value}
      </Text>
    </Card>
  );
}

const todayUTC = () => new Date().toISOString().slice(0, 10);
const daysAgoUTC = (n) =>
  new Date(Date.now() - (n - 1) * 86400000).toISOString().slice(0, 10);

export default function UsagePanel({ scheme }) {
  const [data, setData] = useState(null);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [range, setRange] = useState("today");
  const [model, setModel] = useState("__all__");
  const [profile, setProfile] = useState("__all__");

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

  const allRows = data?.daily || [];

  const modelOpts = useMemo(() => {
    const set = Array.from(new Set(allRows.map((r) => r.model))).sort();
    return [{ value: "__all__", label: "全部模型" }, ...set.map((m) => ({ value: m, label: m }))];
  }, [allRows]);
  const profileOpts = useMemo(() => {
    const set = Array.from(new Set(allRows.map((r) => r.profile))).sort();
    return [{ value: "__all__", label: "全部实例" }, ...set.map((p) => ({ value: p, label: p }))];
  }, [allRows]);

  // 筛选（模型 / 实例 / 时间范围）
  const rows = useMemo(() => {
    let startDate = null;
    if (range === "today") startDate = todayUTC();
    else if (range !== "all") startDate = daysAgoUTC(parseInt(range, 10));
    return allRows.filter((r) => {
      const d = r.datetime.slice(0, 10);
      if (model !== "__all__" && r.model !== model) return false;
      if (profile !== "__all__" && r.profile !== profile) return false;
      if (startDate && d < startDate) return false;
      return true;
    });
  }, [allRows, model, profile, range]);

  const totals = useMemo(() => {
    let i = 0, o = 0, req = 0, cr = 0;
    for (const r of rows) {
      i += r.input; o += r.output; req += r.requests; cr += r.cacheRead;
    }
    return { input: i, output: o, requests: req, cacheRead: cr };
  }, [rows]);

  // 聚合：当天按小时，其它按天
  const series = useMemo(() => {
    const byHour = range === "today";
    const bucket = new Map();
    for (const r of rows) {
      const key = byHour ? r.datetime.slice(11, 13) : r.datetime.slice(0, 10);
      const cur = bucket.get(key) || { input: 0, output: 0, cacheCreate: 0, cacheRead: 0 };
      cur.input += r.input;
      cur.output += r.output;
      cur.cacheCreate += r.cacheCreate;
      cur.cacheRead += r.cacheRead;
      bucket.set(key, cur);
    }
    let labels;
    if (byHour) {
      labels = Array.from({ length: 24 }, (_, h) => String(h).padStart(2, "0"));
    } else {
      labels = Array.from(bucket.keys()).sort();
    }
    const pick = (k) => labels.map((l) => (bucket.get(l) ? bucket.get(l)[k] : 0));
    const disp = byHour ? labels.map((h) => h + ":00") : labels;
    return { labels: disp, input: pick("input"), output: pick("output"), cacheCreate: pick("cacheCreate"), cacheRead: pick("cacheRead") };
  }, [rows, range]);

  const byModel = useMemo(() => {
    const m = new Map();
    for (const r of rows) m.set(r.model, (m.get(r.model) || 0) + r.input + r.output);
    return Array.from(m.entries()).map(([name, value]) => ({ name, value })).sort((a, b) => b.value - a.value);
  }, [rows]);

  const C = lineColors(scheme);
  const mkLine = (name, key) => ({
    name,
    type: "line",
    smooth: true,
    showSymbol: false,
    data: series[key],
    areaStyle: { opacity: 0.1 },
    lineStyle: { width: 2 },
    itemStyle: { color: C[key] },
  });

  const lineOption = useMemo(
    () => ({
      tooltip: { trigger: "axis" },
      legend: { data: ["输入", "输出", "缓存创建", "缓存命中"], top: 0 },
      grid: { left: 52, right: 20, top: 40, bottom: 48 },
      xAxis: { type: "category", boundaryGap: false, data: series.labels, axisLabel: { rotate: range === "today" ? 0 : 30 } },
      yAxis: { type: "value", axisLabel: { formatter: (v) => fmt(v) } },
      series: [
        mkLine("输入", "input"),
        mkLine("输出", "output"),
        mkLine("缓存创建", "cacheCreate"),
        mkLine("缓存命中", "cacheRead"),
      ],
    }),
    [series, scheme, range]
  );

  // 饼图：echarts 默认配色
  const pieOption = useMemo(
    () => ({
      tooltip: { trigger: "item", formatter: (p) => `${p.name}<br/>${fmt(p.value)} (${p.percent}%)` },
      legend: { type: "scroll", bottom: 0 },
      series: [{ type: "pie", radius: ["42%", "70%"], data: byModel, label: { formatter: "{b}" } }],
    }),
    [byModel]
  );

  const hasData = rows.length > 0;

  return (
    <Stack gap="md">
      <Group justify="space-between">
        <Group gap="xs">
          <Title order={5}>用量统计</Title>
          <Badge variant="light" color="gray">本地会话记录</Badge>
        </Group>
        <Button size="xs" variant="light" leftSection={<IconRefresh size={14} />} onClick={load} loading={busy}>
          刷新
        </Button>
      </Group>

      <Card withBorder padding="md" radius="lg">
        <Group gap="sm" align="flex-end" wrap="wrap">
          <Select label="时间范围" data={RANGE_OPTS} value={range} onChange={(v) => setRange(v || "today")} w={170} allowDeselect={false} />
          <Select label="模型" data={modelOpts} value={model} onChange={(v) => setModel(v || "__all__")} w={180} />
          <Select label="实例" data={profileOpts} value={profile} onChange={(v) => setProfile(v || "__all__")} w={160} />
        </Group>
      </Card>

      {err && (
        <Alert color="red" icon={<IconInfoCircle size={16} />} radius="lg">{err}</Alert>
      )}

      {!hasData && !busy && (
        <Card withBorder padding="xl" radius="lg">
          <Stack align="center" gap="xs">
            <IconChartLine size={40} opacity={0.4} />
            <Text c="dimmed" size="sm">当前筛选下没有数据。换个时间范围、或用 claude 跑几次对话后刷新。</Text>
          </Stack>
        </Card>
      )}

      {hasData && (
        <>
          <SimpleGrid cols={{ base: 2, sm: 4 }}>
            <StatCard label="总输入 token" value={fmt(totals.input)} color="brand.7" />
            <StatCard label="总输出 token" value={fmt(totals.output)} />
            <StatCard label="请求次数" value={fmt(totals.requests)} />
            <StatCard label="缓存命中" value={fmt(totals.cacheRead)} />
          </SimpleGrid>

          <Card withBorder padding="md" radius="lg">
            <Text fw={600} mb="xs">
              使用趋势 · {range === "today" ? "当天（按小时）" : RANGE_OPTS.find((r) => r.value === range)?.label}
            </Text>
            <EChart option={lineOption} height={360} />
          </Card>

          <Card withBorder padding="md" radius="lg">
            <Text fw={600} mb="xs">各模型用量占比</Text>
            <EChart option={pieOption} height={300} />
          </Card>
        </>
      )}
    </Stack>
  );
}
