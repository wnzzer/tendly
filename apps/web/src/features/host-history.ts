/**
 * Shapes the recorded resource history for the status bar's trend charts.
 *
 * The server answers a range at one resolution, each point an average and a
 * peak over its bucket. Charts draw the average as a line over a band up to
 * the peak, and break both where buckets are missing, so a stretch with the
 * server stopped reads as a gap rather than a straight line across it.
 */
export interface HostHistoryPoint {
  ts: number;
  cpu_avg: number;
  cpu_max: number;
  memory_used_avg: number;
  memory_used_max: number;
  memory_total: number;
  fan_rpm_avg: number | null;
  fan_rpm_max: number | null;
  temperature_avg: number | null;
  temperature_max: number | null;
  /** The fastest CPU cluster's clock; absent from an older backend. */
  cpu_mhz_avg?: number | null;
  cpu_mhz_max?: number | null;
  /** The whole machine's draw in watts; absent from an older backend. */
  power_avg?: number | null;
  power_max?: number | null;
  /** The busiest GPU's utilization, 0–100; absent from an older backend. */
  gpu_avg?: number | null;
  gpu_max?: number | null;
}
export interface SessionHistoryPoint {
  ts: number;
  session_id: string;
  cpu_avg: number;
  cpu_max: number;
  memory_avg: number;
  memory_max: number;
}
export interface HostHistory {
  /** False when monitoring is turned off; absent from an older backend, which always records. */
  monitoring?: boolean;
  resolution: number;
  host: HostHistoryPoint[];
  sessions: SessionHistoryPoint[];
}

export const RANGES = [
  { key: "1h", seconds: 3600 },
  { key: "24h", seconds: 86400 },
  { key: "7d", seconds: 7 * 86400 },
] as const;
export type RangeKey = (typeof RANGES)[number]["key"];

export interface Sample {
  ts: number;
  avg: number;
  max: number;
}

/** The points that carry a reading for one measure, in time order. */
export function samples(
  points: HostHistoryPoint[],
  pick: (point: HostHistoryPoint) => [number | null, number | null],
): Sample[] {
  const out: Sample[] = [];
  for (const point of points) {
    const [avg, max] = pick(point);
    if (avg == null || max == null || !Number.isFinite(avg) || !Number.isFinite(max)) continue;
    out.push({ ts: point.ts, avg, max: Math.max(avg, max) });
  }
  return out.sort((a, b) => a.ts - b.ts);
}

export const measures = {
  cpu: (p: HostHistoryPoint): [number, number] => [p.cpu_avg, p.cpu_max],
  memory: (p: HostHistoryPoint): [number | null, number | null] =>
    p.memory_total > 0
      ? [(p.memory_used_avg / p.memory_total) * 100, (p.memory_used_max / p.memory_total) * 100]
      : [null, null],
  temperature: (p: HostHistoryPoint): [number | null, number | null] => [p.temperature_avg, p.temperature_max],
  fan: (p: HostHistoryPoint): [number | null, number | null] => [p.fan_rpm_avg, p.fan_rpm_max],
  clock: (p: HostHistoryPoint): [number | null, number | null] => [p.cpu_mhz_avg ?? null, p.cpu_mhz_max ?? null],
  power: (p: HostHistoryPoint): [number | null, number | null] => [p.power_avg ?? null, p.power_max ?? null],
  gpu: (p: HostHistoryPoint): [number | null, number | null] => [p.gpu_avg ?? null, p.gpu_max ?? null],
};

/** Watts, with a decimal while they are few. */
export function watts(value: number): string {
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} W`;
}

export function gigahertz(mhz: number): string {
  return `${(mhz / 1000).toFixed(mhz >= 10_000 ? 0 : 1)} GHz`;
}

/** A round top for an axis that starts at zero: 1, 2 or 5 times a power of ten. */
export function niceCeiling(value: number, floor = 1): number {
  const target = Math.max(value, floor, Number.MIN_VALUE);
  const power = 10 ** Math.floor(Math.log10(target));
  return [1, 2, 5, 10].map(step => step * power).find(step => step >= target) ?? 10 * power;
}

/**
 * A lower and upper bound for an axis that need not start at zero, such as a
 * temperature: the data's range widened to whole steps and to at least
 * `minSpan`, so a steady reading is not drawn as a wild one.
 */
export function niceRange(points: Sample[], step: number, minSpan: number): { bottom: number; top: number } {
  if (!points.length) return { bottom: 0, top: minSpan };
  const low = Math.min(...points.map(point => point.avg)),
    high = Math.max(...points.map(point => point.max));
  let bottom = Math.max(0, Math.floor(low / step) * step),
    top = Math.ceil(high / step) * step;
  if (top - bottom < minSpan) {
    const grow = minSpan - (top - bottom);
    bottom = Math.max(0, bottom - Math.ceil(grow / 2 / step) * step);
    top = Math.max(top, bottom + minSpan);
  }
  return { bottom, top: top > bottom ? top : bottom + step };
}

/**
 * Points gathered into at most `bins` equal buckets across the window, each
 * the mean of its averages and the highest of its peaks, so a chart a few
 * hundred pixels wide is not asked to draw several samples per pixel. Empty
 * buckets stay empty, so gaps survive. Points already sparser than a bucket
 * are returned as they are.
 */
export function bin(
  points: Sample[],
  from: number,
  to: number,
  bins: number,
  resolution: number,
): { points: Sample[]; resolution: number } {
  const width = (to - from) / Math.max(1, bins);
  if (width <= resolution) return { points, resolution };
  const buckets = new Map<number, { sum: number; count: number; max: number }>();
  for (const point of points) {
    const index = Math.floor((point.ts - from) / width);
    const bucket = buckets.get(index) ?? { sum: 0, count: 0, max: -Infinity };
    bucket.sum += point.avg;
    bucket.count += 1;
    bucket.max = Math.max(bucket.max, point.max);
    buckets.set(index, bucket);
  }
  return {
    points: [...buckets]
      .sort(([a], [b]) => a - b)
      .map(([index, bucket]) => ({
        ts: from + (index + 0.5) * width,
        avg: bucket.sum / bucket.count,
        max: bucket.max,
      })),
    resolution: width,
  };
}

/** Runs of points no further apart than a missing bucket allows. */
export function segments(points: Sample[], resolution: number): Sample[][] {
  const runs: Sample[][] = [];
  for (const point of points) {
    const run = runs.at(-1);
    if (run && point.ts - run.at(-1)!.ts <= resolution * 2.5) run.push(point);
    else runs.push([point]);
  }
  return runs;
}

/** The plotted window; values run from `bottom` (zero when absent) to `top`. */
export interface Frame {
  from: number;
  to: number;
  width: number;
  height: number;
  top: number;
  bottom?: number;
}

/**
 * SVG path data for the average line and the band up to the peak. A run of
 * one point becomes a short flat stroke so a lone reading is still visible.
 */
export function chartPaths(points: Sample[], resolution: number, frame: Frame): { line: string; band: string } {
  const span = Math.max(1, frame.to - frame.from);
  const x = (ts: number) => +(((ts - frame.from) / span) * frame.width).toFixed(2);
  const bottom = frame.bottom ?? 0,
    range = Math.max(Number.MIN_VALUE, frame.top - bottom);
  const y = (value: number) =>
    +(frame.height - ((Math.min(Math.max(value, bottom), frame.top) - bottom) / range) * frame.height).toFixed(2);
  const half = Math.max(0.75, ((resolution / span) * frame.width) / 2);
  let line = "",
    band = "";
  for (const run of segments(points, resolution)) {
    const xs = run.length === 1 ? [x(run[0].ts) - half, x(run[0].ts) + half] : run.map(p => x(p.ts));
    const values = run.length === 1 ? [run[0], run[0]] : run;
    line += values.map((p, i) => `${i ? "L" : "M"}${xs[i]} ${y(p.avg)}`).join("");
    band +=
      values.map((p, i) => `${i ? "L" : "M"}${xs[i]} ${y(p.max)}`).join("") +
      values
        .map((p, i) => [p, i] as const)
        .reverse()
        .map(([p, i]) => `L${xs[i]} ${y(p.avg)}`)
        .join("") +
      "Z";
  }
  return { line, band };
}

/** The point closest in time to `ts`, if one lies within a bucket of it. */
export function nearest(points: Sample[], ts: number, resolution: number): Sample | undefined {
  let best: Sample | undefined;
  for (const point of points) if (!best || Math.abs(point.ts - ts) < Math.abs(best.ts - ts)) best = point;
  return best && Math.abs(best.ts - ts) <= resolution * 1.5 ? best : undefined;
}

export interface SessionSummary {
  session_id: string;
  cpu_avg: number;
  cpu_max: number;
  memory_max: number;
}

/**
 * Sessions ranked by how much CPU they used over the range, as a share of the
 * whole machine like the host figure. A session counts only the buckets it
 * was running in, so one that ran briefly but hard still ranks high.
 */
export function busiestSessions(points: SessionHistoryPoint[], cores: number, limit = 5): SessionSummary[] {
  const by = new Map<string, { sum: number; count: number; max: number; memory: number }>();
  for (const point of points) {
    const entry = by.get(point.session_id) ?? { sum: 0, count: 0, max: 0, memory: 0 };
    entry.sum += point.cpu_avg;
    entry.count += 1;
    entry.max = Math.max(entry.max, point.cpu_max);
    entry.memory = Math.max(entry.memory, point.memory_max);
    by.set(point.session_id, entry);
  }
  const share = Math.max(1, cores);
  return [...by]
    .map(([session_id, entry]) => ({
      session_id,
      cpu_avg: entry.sum / entry.count / share,
      cpu_max: entry.max / share,
      memory_max: entry.memory,
    }))
    .sort((a, b) => b.cpu_avg - a.cpu_avg || b.memory_max - a.memory_max)
    .slice(0, limit);
}
