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
  Popover,
} from "@mantine/core";
import { DatePicker } from "@mantine/dates";
import "@mantine/dates/styles.css";
import { IconRefresh, IconChartLine, IconInfoCircle, IconCalendar } from "@tabler/icons-react";
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

const pad = (n) => String(n).padStart(2, "0");
const dstr = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
// UTC 的 "2026-06-22T04" → 本地 { date, hour }
const toLocal = (dt) => {
  const d = new Date(dt + ":00:00Z");
  return { date: dstr(d), hour: d.getHours() };
};
const todayLocal = () => dstr(new Date());
const daysAgoLocal = (n) => dstr(new Date(Date.now() - (n - 1) * 86400000));

// 折线配色（不随主题，固定好看的渐变）
const SERIES = [
  { key: "input", name: "输入", color: "#3b82f6" },
  { key: "output", name: "输出", color: "#10b981" },
  { key: "cacheCreate", name: "缓存创建", color: "#f59e0b" },
  { key: "cacheRead", name: "缓存命中", color: "#06b6d4" },
];

// 卡片配色（各不相同）
const CARDS = [
  { key: "input", label: "总输入 token", bg: "#eef4ff", fg: "#2563eb" },
  { key: "output", label: "总输出 token", bg: "#eafaf2", fg: "#059669" },
  { key: "requests", label: "请求次数", bg: "#fff4e8", fg: "#d97706" },
  { key: "cacheRead", label: "缓存命中", bg: "#e9f8fb", fg: "#0891b2" },
];

const QUICK = [
  { value: "today", label: "当天" },
  { value: "7", label: "近 7 天" },
  { value: "14", label: "近 14 天" },
  { value: "30", label: "近 30 天" },
  { value: "all", label: "全部" },
];

function StatCard({ label, value, bg, fg }) {
  return (
    <Card withBorder padding="md" radius="lg" style={{ background: bg }}>
      <Text size="xs" style={{ color: fg, opacity: 0.85 }}>
        {label}
      </Text>
      <Text fw={800} size="xl" style={{ color: fg }}>
        {value}
      </Text>
    </Card>
  );
}

export default function UsagePanel() {
  const [data, setData] = useState(null);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [range, setRange] = useState({ kind: "today" });
  const [custom, setCustom] = useState([null, null]);
  const [pop, setPop] = useState(false);
  const [model, setModel] = useState("__all__");
  const [profile, setProfile] = useState("__all__");

  const load = () => {
    setBusy(true);
    setErr("");
    api.usageStats().then(setData).catch((e) => setErr(String(e))).finally(() => setBusy(false));
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

  // 计算有效起止（本地日期）
  const bounds = useMemo(() => {
    if (range.kind === "today") return { start: todayLocal(), end: todayLocal() };
    if (range.kind === "all") return { start: null, end: null };
    if (range.kind === "custom") {
      const s = custom[0] ? dstr(custom[0]) : null;
      const e = custom[1] ? dstr(custom[1]) : s;
      return { start: s, end: e };
    }
    return { start: daysAgoLocal(parseInt(range.kind, 10)), end: todayLocal() };
  }, [range, custom]);

  // 是否按小时（当天，或自定义单日）
  const byHour = bounds.start && bounds.start === bounds.end;

  // 预计算每行的本地 date/hour，并筛选
  const rows = useMemo(() => {
    return allRows
      .map((r) => ({ ...r, ...toLocal(r.datetime) }))
      .filter((r) => {
        if (model !== "__all__" && r.model !== model) return false;
        if (profile !== "__all__" && r.profile !== profile) return false;
        if (bounds.start && r.date < bounds.start) return false;
        if (bounds.end && r.date > bounds.end) return false;
        return true;
      });
  }, [allRows, model, profile, bounds]);

  const totals = useMemo(() => {
    let input = 0, output = 0, requests = 0, cacheRead = 0;
    for (const r of rows) {
      input += r.input; output += r.output; requests += r.requests; cacheRead += r.cacheRead;
    }
    return { input, output, requests, cacheRead };
  }, [rows]);

  const series = useMemo(() => {
    const bucket = new Map();
    for (const r of rows) {
      const key = byHour ? pad(r.hour) : r.date;
      const cur = bucket.get(key) || { input: 0, output: 0, cacheCreate: 0, cacheRead: 0 };
      cur.input += r.input; cur.output += r.output;
      cur.cacheCreate += r.cacheCreate; cur.cacheRead += r.cacheRead;
      bucket.set(key, cur);
    }
    let labels;
    if (byHour) labels = Array.from({ length: 24 }, (_, h) => pad(h));
    else labels = Array.from(bucket.keys()).sort();
    const pick = (k) => labels.map((l) => (bucket.get(l) ? bucket.get(l)[k] : 0));
    const disp = byHour ? labels.map((h) => h + ":00") : labels;
    return { labels: disp, input: pick("input"), output: pick("output"), cacheCreate: pick("cacheCreate"), cacheRead: pick("cacheRead") };
  }, [rows, byHour]);

  const byModel = useMemo(() => {
    const m = new Map();
    for (const r of rows) m.set(r.model, (m.get(r.model) || 0) + r.input + r.output);
    return Array.from(m.entries()).map(([name, value]) => ({ name, value })).sort((a, b) => b.value - a.value);
  }, [rows]);

  const lineOption = useMemo(() => {
    const mkArea = (color) => ({
      color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
        { offset: 0, color: color + "55" },
        { offset: 1, color: color + "05" },
      ]),
    });
    return {
      tooltip: { trigger: "axis" },
      legend: { data: SERIES.map((s) => s.name), top: 0 },
      grid: { left: 56, right: 22, top: 40, bottom: 48 },
      xAxis: { type: "category", boundaryGap: false, data: series.labels, axisLabel: { rotate: byHour ? 0 : 30 } },
      yAxis: { type: "value", axisLabel: { formatter: (v) => fmt(v) } },
      series: SERIES.map((s) => ({
        name: s.name,
        type: "line",
        smooth: true,
        showSymbol: false,
        data: series[s.key],
        lineStyle: { width: 2.5, color: s.color },
        itemStyle: { color: s.color },
        areaStyle: mkArea(s.color),
      })),
    };
  }, [series, byHour]);

  const pieOption = useMemo(() => ({
    tooltip: { trigger: "item", formatter: (p) => `${p.name}<br/>${fmt(p.value)} (${p.percent}%)` },
    legend: { type: "scroll", bottom: 0 },
    series: [{ type: "pie", radius: ["42%", "70%"], data: byModel, label: { formatter: "{b}" } }],
  }), [byModel]);

  const rangeLabel = useMemo(() => {
    if (range.kind === "custom" && custom[0]) {
      const s = dstr(custom[0]).slice(5);
      const e = custom[1] ? dstr(custom[1]).slice(5) : s;
      return `${s} ~ ${e}`;
    }
    return QUICK.find((q) => q.value === range.kind)?.label || "当天";
  }, [range, custom]);

  const pickQuick = (v) => {
    setRange({ kind: v });
    setCustom([null, null]);
    setPop(false);
  };

  const hasData = rows.length > 0;

  return (
    <Stack gap="md">
      <Group justify="space-between">
        <Group gap="xs">
          <Title order={5}>用量统计</Title>
          <Badge variant="light" color="gray">本地会话记录（已按本地时间）</Badge>
        </Group>
        <Button size="xs" variant="light" leftSection={<IconRefresh size={14} />} onClick={load} loading={busy}>
          刷新
        </Button>
      </Group>

      <Card withBorder padding="md" radius="lg">
        <Group gap="sm" align="flex-end" wrap="wrap">
          <div>
            <Text size="sm" fw={500} mb={4}>时间范围</Text>
            <Popover opened={pop} onChange={setPop} position="bottom-start" shadow="md" withinPortal>
              <Popover.Target>
                <Button variant="default" leftSection={<IconCalendar size={16} />} onClick={() => setPop((o) => !o)}>
                  {rangeLabel}
                </Button>
              </Popover.Target>
              <Popover.Dropdown>
                <Stack gap="sm">
                  <Group gap={6}>
                    {QUICK.map((q) => (
                      <Button
                        key={q.value}
                        size="xs"
                        variant={range.kind === q.value ? "filled" : "light"}
                        onClick={() => pickQuick(q.value)}
                      >
                        {q.label}
                      </Button>
                    ))}
                  </Group>
                  <Text size="xs" c="dimmed">或在日历选自定义范围：</Text>
                  <DatePicker
                    type="range"
                    value={custom}
                    onChange={(v) => {
                      setCustom(v);
                      if (v[0] && v[1]) {
                        setRange({ kind: "custom" });
                        setPop(false);
                      }
                    }}
                  />
                </Stack>
              </Popover.Dropdown>
            </Popover>
          </div>
          <Select label="模型" data={modelOpts} value={model} onChange={(v) => setModel(v || "__all__")} w={180} />
          <Select label="实例" data={profileOpts} value={profile} onChange={(v) => setProfile(v || "__all__")} w={160} />
        </Group>
      </Card>

      {err && <Alert color="red" icon={<IconInfoCircle size={16} />} radius="lg">{err}</Alert>}

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
            {CARDS.map((c) => (
              <StatCard key={c.key} label={c.label} value={fmt(totals[c.key])} bg={c.bg} fg={c.fg} />
            ))}
          </SimpleGrid>

          <Card withBorder padding="md" radius="lg">
            <Text fw={600} mb="xs">使用趋势 · {rangeLabel}{byHour ? "（按小时）" : ""}</Text>
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
