/** Figures for the system page: sizes, rates, durations and short trend lines. */

export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  const index = Math.min(units.length - 1, Math.floor(Math.log(value) / Math.log(1024)));
  const scaled = value / 1024 ** index;
  return `${scaled >= 100 || index === 0 ? Math.round(scaled) : scaled.toFixed(1)} ${units[index]}`;
}

export function formatRate(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`;
}

/** Days and hours, or hours and minutes, or minutes: the two largest units. */
export function formatDuration(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const days = Math.floor(s / 86400),
    hours = Math.floor((s % 86400) / 3600),
    minutes = Math.floor((s % 3600) / 60);
  if (days) return `${days}d ${hours}h`;
  if (hours) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

/** Calm under 70%, a warning to 90%, then danger. */
export function levelFor(percent: number): "ok" | "warn" | "danger" {
  return percent >= 90 ? "danger" : percent >= 70 ? "warn" : "ok";
}

/** About two minutes at one reading every two seconds. */
export const HISTORY_LENGTH = 60;

export function pushSample(series: readonly number[], value: number, length = HISTORY_LENGTH): number[] {
  const next = [...series, Number.isFinite(value) ? value : 0];
  return next.length > length ? next.slice(next.length - length) : next;
}

/**
 * An SVG path across a 100×28 box. With `max` the scale is fixed (a percent);
 * without it the line fills the box to the series' own peak (a rate).
 */
export function sparkPath(series: readonly number[], max?: number, length = HISTORY_LENGTH): string {
  if (series.length < 2) return "";
  const top = Math.max(max ?? Math.max(...series), 1e-9);
  const step = 100 / (length - 1),
    offset = (length - series.length) * step;
  return series
    .map((value, index) => {
      const x = offset + index * step;
      const y = 28 - Math.min(1, Math.max(0, value / top)) * 26 - 1;
      return `${index ? "L" : "M"}${x.toFixed(2)} ${y.toFixed(2)}`;
    })
    .join(" ");
}
