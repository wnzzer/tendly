import {
  cacheHitRate,
  cacheSavings,
  formatCost,
  formatHours,
  formatTokens,
  trend,
  type Allowance,
  type UsageBucket,
  type UsageProvider,
  type UsageReport,
} from "./usage-model";

/**
 * The usage page's figures, worked out from a report: the allowance cards,
 * the ten tiles, the stacked daily chart, the weekday-by-hour heat map and
 * the few sentences on what stands out. Plain functions of the report, so
 * the page only holds what is chosen and what is loaded.
 */

type Translate = (text: string, values?: Record<string, string | number>) => string;

export interface AllowanceCard {
  id: string;
  name: string;
  provider: UsageProvider;
  plan: string | null;
  windows: Allowance[];
}

/** One card per account, its windows side by side, shortest first. */
export function allowanceCards(rows: readonly Allowance[]): AllowanceCard[] {
  const byId = new Map<string, AllowanceCard>();
  for (const row of rows) {
    const entry = byId.get(row.account_id) ?? {
      id: row.account_id,
      name: row.account,
      provider: row.provider,
      plan: row.plan,
      windows: [],
    };
    entry.windows.push(row);
    byId.set(row.account_id, entry);
  }
  return [...byId.values()].map(entry => ({
    ...entry,
    windows: entry.windows.sort((a, b) => a.window_minutes - b.window_minutes),
  }));
}

/** "5h", "7d", or the hours in between, for a window's length in minutes. */
export function windowLabel(minutes: number): string {
  if (minutes >= 7 * 24 * 60 - 60) return "7d";
  if (minutes <= 5 * 60 + 30) return "5h";
  return `${Math.round(minutes / 60)}h`;
}

/** A change in percent, signed, or nothing where there is nothing to compare. */
export function changeLabel(value?: number): string {
  return value === undefined ? "" : `${value >= 0 ? "+" : ""}${value.toFixed(1)}%`;
}

export interface UsageTile {
  key: string;
  label: string;
  value: string;
  change?: number;
  tone?: "cost" | "quiet" | "time";
  hint?: string;
}

/** The ten figures: what it cost, what it was made of, and how much work it was. */
export function usageTiles(data: UsageReport, t: Translate): UsageTile[] {
  const now = data.totals,
    before = data.previous;
  const cache = (bucket: UsageBucket) => bucket.cache_read + bucket.cache_write_5m + bucket.cache_write_1h;
  const messages = (bucket: UsageBucket) => bucket.calls + bucket.user_messages;
  const pricedAt = data.prices_updated_at ? new Date(data.prices_updated_at) : undefined;
  const listPrices =
    pricedAt && !Number.isNaN(pricedAt.getTime())
      ? t("At API list prices, updated {date}", { date: pricedAt.toLocaleDateString() })
      : t("At API list prices");
  return [
    {
      key: "cost",
      label: "Estimated cost",
      value: formatCost(now.cost),
      change: trend(now.cost, before.cost),
      tone: "cost",
      hint: now.unpriced
        ? t("{tokens} tokens on unpriced models are not included", { tokens: formatTokens(now.unpriced) })
        : listPrices,
    },
    { key: "total", label: "Total tokens", value: formatTokens(now.total), change: trend(now.total, before.total) },
    {
      key: "input",
      label: "Input tokens",
      value: formatTokens(now.input),
      change: trend(now.input, before.input),
      hint: t("Input the cache did not serve"),
    },
    {
      key: "output",
      label: "Output tokens",
      value: formatTokens(now.output),
      change: trend(now.output, before.output),
    },
    {
      key: "cache",
      label: "Cache tokens",
      value: formatTokens(cache(now)),
      change: trend(cache(now), cache(before)),
      tone: "quiet",
      hint: t("{rate}% hit rate, about {cost} saved", {
        rate: Math.round(cacheHitRate(now)),
        cost: formatCost(cacheSavings(data.models)),
      }),
    },
    {
      key: "active",
      label: "Active time",
      value: formatHours(now.active_minutes),
      change: trend(now.active_minutes, before.active_minutes),
      tone: "time",
      hint: t("Five-minute stretches with any activity"),
    },
    {
      key: "duration",
      label: "Total duration",
      value: formatHours(data.duration_seconds / 60),
      change: trend(data.duration_seconds, data.previous_duration_seconds),
      hint: t("Each session from its first event to its last"),
    },
    {
      key: "sessions",
      label: "Sessions",
      value: data.conversation_count.toLocaleString(),
      change: undefined,
      hint: undefined,
    },
    {
      key: "messages",
      label: "Total messages",
      value: messages(now).toLocaleString(),
      change: trend(messages(now), messages(before)),
      hint: t("Model replies and your messages"),
    },
    {
      key: "user",
      label: "Your messages",
      value: now.user_messages.toLocaleString(),
      change: trend(now.user_messages, before.user_messages),
      hint: undefined,
    },
  ];
}

export type Metric = "tokens" | "cost" | "time";
export const SERIES = [
  { key: "output", label: "Output", shade: 92 },
  { key: "input", label: "Input", shade: 52 },
  { key: "cache", label: "Cache", shade: 22 },
] as const;
export type SeriesKey = (typeof SERIES)[number]["key"];

export function seriesValue(bucket: UsageBucket, key: SeriesKey): number {
  if (key === "output") return bucket.output;
  if (key === "input") return bucket.input;
  return bucket.cache_read + bucket.cache_write_5m + bucket.cache_write_1h;
}

/** The ink at `percent` strength: the chart is drawn in one hue from faint to full. */
export const shade = (percent: number) => `color-mix(in srgb, var(--ink) ${percent}%, transparent)`;

export interface DayPart {
  key: string;
  label: string;
  value: number;
  color: string;
}

/** What one day's bar is stacked from: cost or active time alone, or the token kinds that are shown. */
export function dayParts(day: UsageBucket, metric: Metric, shown: Readonly<Record<SeriesKey, boolean>>): DayPart[] {
  if (metric === "cost") return [{ key: "cost", label: "Cost", value: day.cost, color: "var(--ok)" }];
  if (metric === "time")
    return [{ key: "time", label: "Active time", value: day.active_minutes, color: "var(--info)" }];
  return SERIES.filter(series => shown[series.key]).map(series => ({
    key: series.key,
    label: series.label,
    value: seriesValue(day, series.key),
    color: shade(series.shade),
  }));
}

export interface BarRect extends DayPart {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * The rectangles of one day's stacked bar in a chart `width` by `height`,
 * scaled to the tallest day. Each slot is shared evenly; a bar is 3 to 22
 * wide and centred in it, and a gap is left between parts tall enough to show one.
 */
export function stackedBar(
  parts: readonly DayPart[],
  index: number,
  count: number,
  max: number,
  width: number,
  height: number,
): BarRect[] {
  const slot = width / Math.max(1, count),
    barWidth = Math.max(3, Math.min(22, slot - 4));
  const x = index * slot + (slot - barWidth) / 2;
  let y = height;
  return parts
    .map(part => {
      const partHeight = (part.value / max) * height;
      y -= partHeight;
      return { ...part, x, y, width: barWidth, height: Math.max(0, partHeight - (partHeight > 3 ? 1.5 : 0)) };
    })
    .filter(rect => rect.height > 0);
}

/** "2026-10-07" as "10/7". */
export function dateLabel(date: string): string {
  const [, month, day] = date.split("-");
  return `${Number(month)}/${Number(day)}`;
}

/** A heat map cell's colour: faint when empty, then by square root so a few busy hours do not wash out the rest. */
export function heatShade(value: number, max: number): string {
  return value > 0 ? shade(Math.round(14 + Math.sqrt(value / max) * 82)) : shade(5);
}

/** The weekday (0 = Monday, as the heat map rows run) and hour with the most active time, if any. */
export function busiestHour(heatmap: readonly (readonly number[])[]): { day: number; hour: number } | undefined {
  let best = { day: 0, hour: 0, value: 0 };
  heatmap.forEach((row, day) =>
    row.forEach((value, hour) => {
      if (value > best.value) best = { day, hour, value };
    }),
  );
  return best.value ? { day: best.day, hour: best.hour } : undefined;
}

/** A few plain sentences on what stands out. */
export function usageInsights(data: UsageReport, t: Translate, weekdays: readonly string[]): string[] {
  if (!data.totals.total) return [];
  const lines: string[] = [];
  const top = data.models[0];
  if (top)
    lines.push(
      t("{model} did {share}% of the work.", {
        model: top.model,
        share: Math.round((top.total / data.totals.total) * 100),
      }),
    );
  const peak = [...data.days].sort((a, b) => b.total - a.total)[0];
  if (peak?.total)
    lines.push(
      t("The busiest day was {date}, with {tokens} tokens.", { date: peak.date, tokens: formatTokens(peak.total) }),
    );
  const busiest = busiestHour(data.heatmap_active);
  if (busiest)
    lines.push(
      t("You work hardest on {weekday} around {hour}:00.", { weekday: weekdays[busiest.day]!, hour: busiest.hour }),
    );
  if (data.totals.user_messages)
    lines.push(
      t("Each of your messages led to {calls} model calls on average.", {
        calls: (data.totals.calls / data.totals.user_messages).toFixed(1),
      }),
    );
  return lines;
}
