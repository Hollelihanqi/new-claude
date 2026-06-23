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
  Select,
} from "@mantine/core";
import { DatePickerInput } from "@mantine/dates";
import "@mantine/dates/styles.css";
import dayjs from "dayjs";
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

function weekStart(dateStr) {
  const d = new Date(dateStr + "T00:00:00Z");
  const day = (d.getUTCDay() + 6) % 7;
  d.setUTCDate(d.getUTCDate() - day);
  return d.toISOString().slice(0, 10);
}

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

const LINE_COLORS = {
  input: "#ff6600",
  output: "#1d9e75",
  cacheCreate: "#e8950c",
  cacheRead: "#3b82f6",
};

const REFRESH_OPTS = [
  { value: "0", label: "不自动刷新" },
  { value: "10", label: "每 10 秒" },
  { value: "30", label: "每 30 秒" },
  { value: "60", label: "每 60 秒" },
];

export default function UsagePanel() {
  const [data, setData] = useState(null);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [gran, setGran] = useState("day");
  const [model, setModel] = useState("__all__");
  const [profile, setProfile] = useState("__all__");
  const [range, setRange] = useState([null, null]);
  const [refreshSec, setRefreshSec] = useState("0");

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

  // 自动刷新
  useEffect(() => {
    const sec = parseInt(refreshSec, 10);
    if (!sec) return;
    const t = setInterval(load, sec * 1000);
    return () => clearInterval(t);
  }, [refreshSec]);

  const allRows = data?.daily || [];

  // 可选项
  const modelOpts = useMemo(() => {
    const set = Array.from(new Set(allRows.map((r) => r.model))).sort();
    return [{ value: "__all__", label: "全部模型" }, ...set.map((m) => ({ value: m, label: m }))];
  }, [allRows]);
  const profileOpts = useMemo(() => {
    const set = Array.from(new Set(allRows.map((r) => r.profile))).sort();
    return [{ value: "__all__", label: "全部实例" }, ...set.map((p) => ({ value: p, label: p }))];
  }, [allRows]);

  // 应用筛选（模型 / 实例 / 日期范围）
  const rows = useMemo(() => {
    const [start, end] = range;
    const s = start ? dayjs(start).format("YYYY-MM-DD") : null;
    const e = end ? dayjs(end).format("YYYY-MM-DD") : null;
    return allRows.filter((r) => {
      if (model !== "__all__" && r.model !== model) return false;
      if (profile !== "__all__" && r.profile !== profile) return false;
      if (s && r.date < s) return false;
      if (e && r.date > e) return false;
      return true;
    });
  }, [allRows, model, profile, range]);

  const totals = useMemo(() => {
    let i = 0, o = 0, req = 0, cc = 0, cr = 0;
    for (const r of rows) {
      i += r.input; o += r.output; req += r.requests;
      cc += r.cacheCreate; cr += r.cacheRead;
    }
    return { input: i, output: o, requests: req, cacheCreate: cc, cacheRead: cr };
  }, [rows]);

  const series = useMemo(() => {
    const bucket = new Map();
    for (const r of rows) {
      let key = r.date;
      if (gran === "week") key = weekStart(r.date);
      else if (gran === "month") key = r.date.slice(0, 7);
      const cur = bucket.get(key) || { input: 0, output: 0, cacheCreate: 0, cacheRead: 0 };
      cur.input += r.input;
      cur.output += r.output;
      cur.cacheCreate += r.cacheCreate;
      cur.cacheRead += r.cacheRead;
      bucket.set(key, cur);
    }
    const labels = Array.from(bucket.keys()).sort();
    const pick = (k) => labels.map((l) => bucket.get(l)[k]);
    return { labels, input: pick("input"), output: pick("output"), cacheCreate: pick("cacheCreate"), cacheRead: pick("cacheRead") };
  }, [rows, gran]);

  const byModel = useMemo(() => {
    const m = new Map();
    for (const r of rows) m.set(r.model, (m.get(r.model) || 0) + r.input + r.output);
    return Array.from(m.entries()).map(([name, value]) => ({ name, value })).sort((a, b) => b.value - a.value);
  }, [rows]);

  const mkLine = (name, key) => ({
    name,
    type: "line",
    smooth: true,
    showSymbol: false,
    data: series[key],
    areaStyle: { opacity: 0.1 },
    lineStyle: { width: 2 },
    itemStyle: { color: LINE_COLORS[key] },
  });

  const lineOption = useMemo(
    () => ({
      tooltip: { trigger: "axis" },
      legend: { data: ["输入", "输出", "缓存创建", "缓存命中"], top: 0 },
      grid: { left: 52, right: 20, top: 40, bottom: 48 },
      xAxis: { type: "category", boundaryGap: false, data: series.labels, axisLabel: { rotate: 30 } },
      yAxis: { type: "value", axisLabel: { formatter: (v) => fmt(v) } },
      series: [
        mkLine("输入", "input"),
        mkLine("输出", "output"),
        mkLine("缓存创建", "cacheCreate"),
        mkLine("缓存命中", "cacheRead"),
      ],
    }),
    [series]
  );

  const pieOption = useMemo(
    () => ({
      tooltip: { trigger: "item", formatter: (p) => `${p.name}<br/>${fmt(p.value)} (${p.percent}%)` },
      legend: { type: "scroll", bottom: 0 },
      color: ["#ff6600", "#ff8a3d", "#ffa166", "#e8950c", "#1d9e75", "#3b82f6"],
      series: [{ type: "pie", radius: ["42%", "70%"], data: byModel, label: { formatter: "{b}" } }],
    }),
    [byModel]
  );

  const setQuick = (days) => {
    if (days === 0) {
      const today = new Date();
      setRange([today, today]);
    } else {
      const end = new Date();
      const start = new Date();
      start.setDate(start.getDate() - (days - 1));
      setRange([start, end]);
    }
  };

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
        <Button size="xs" variant="light" leftSection={<IconRefresh size={14} />} onClick={load} loading={busy}>
          刷新
        </Button>
      </Group>

      {/* 筛选条 */}
      <Card withBorder padding="md" radius="lg">
        <Group gap="sm" align="flex-end" wrap="wrap">
          <Select label="模型" data={modelOpts} value={model} onChange={(v) => setModel(v || "__all__")} w={180} />
          <Select label="实例" data={profileOpts} value={profile} onChange={(v) => setProfile(v || "__all__")} w={160} />
          <DatePickerInput
            type="range"
            label="日期范围"
            placeholder="选择起止日期"
            value={range}
            onChange={setRange}
            clearable
            w={240}
            valueFormat="MM/DD"
          />
          <Select
            label="自动刷新"
            data={REFRESH_OPTS}
            value={refreshSec}
            onChange={(v) => setRefreshSec(v || "0")}
            w={140}
          />
          <Button.Group>
            <Button size="sm" variant="default" onClick={() => setQuick(0)}>当天</Button>
            <Button size="sm" variant="default" onClick={() => setQuick(7)}>7 天</Button>
            <Button size="sm" variant="default" onClick={() => setQuick(14)}>14 天</Button>
            <Button size="sm" variant="default" onClick={() => setQuick(30)}>30 天</Button>
            <Button size="sm" variant="default" onClick={() => setRange([null, null])}>全部</Button>
          </Button.Group>
        </Group>
      </Card>

      {err && (
        <Alert color="red" icon={<IconInfoCircle size={16} />} radius="lg">
          {err}
        </Alert>
      )}

      <Alert variant="light" color="hermes" icon={<IconInfoCircle size={16} />} radius="lg">
        数据来自本机 Claude Code 的会话记录，统计你在这台电脑上的 token 用量；失败/未连通的请求不计入。
      </Alert>

      {!hasData && !busy && (
        <Card withBorder padding="xl" radius="lg">
          <Stack align="center" gap="xs">
            <IconChartLine size={40} opacity={0.4} />
            <Text c="dimmed" size="sm">
              当前筛选下没有数据。换个日期范围、或用 claude 跑几次对话后刷新。
            </Text>
          </Stack>
        </Card>
      )}

      {hasData && (
        <>
          <SimpleGrid cols={{ base: 2, sm: 4 }}>
            <StatCard label="总输入 token" value={fmt(totals.input)} color="hermes.7" />
            <StatCard label="总输出 token" value={fmt(totals.output)} color="teal" />
            <StatCard label="请求次数" value={fmt(totals.requests)} />
            <StatCard label="缓存命中" value={fmt(totals.cacheRead)} color="blue" />
          </SimpleGrid>

          <Card withBorder padding="md" radius="lg">
            <Group justify="space-between" mb="xs">
              <Text fw={600}>使用趋势</Text>
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
            <EChart option={lineOption} height={360} />
          </Card>

          <Card withBorder padding="md" radius="lg">
            <Text fw={600} mb="xs">
              各模型用量占比
            </Text>
            <EChart option={pieOption} height={300} />
          </Card>
        </>
      )}
    </Stack>
  );
}
