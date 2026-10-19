import type { AgentProviderKind } from "@tendly/protocol";

/** The usage report as the server sends it, and the figures the page derives from it. */
export type UsageProvider = AgentProviderKind;
export interface UsageBucket {
  input: number;
  cache_read: number;
  cache_write_5m: number;
  cache_write_1h: number;
  output: number;
  reasoning: number;
  total: number;
  cost: number;
  unpriced: number;
  calls: number;
  user_messages: number;
  active_minutes: number;
  /** Requests answered and failed, from clients that log failures; absent from an older backend. */
  answered?: number;
  failed?: number;
  /** Attempts that failed and were tried again. */
  retried?: number;
  /** Output of the calls whose duration is known, and that duration. */
  timed_output?: number;
  timed_ms?: number;
}
export interface Price { input: number; output: number; cache_read: number }
export interface UsageModel extends UsageBucket { model: string; provider: UsageProvider; price: Price | null; custom_price: boolean }
export interface UsageConversation extends UsageBucket {
  conversation: string;
  provider: UsageProvider;
  session_id: string | null;
  title: string | null;
  cwd: string;
  models: string[];
  first_at: number;
  last_at: number;
}
export interface UsageReport {
  from: number;
  to: number;
  totals: UsageBucket;
  previous: UsageBucket;
  days: Array<UsageBucket & { date: string }>;
  models: UsageModel[];
  providers: UsageModel[];
  heatmap: number[][];
  heatmap_cost: number[][];
  heatmap_active: number[][];
  duration_seconds: number;
  previous_duration_seconds: number;
  options: { providers: UsageProvider[]; models: string[]; projects: string[] };
  conversations: UsageConversation[];
  conversation_count: number;
  directories: Array<[string, number]>;
  /** When the published prices were last updated; absent from an older backend. */
  prices_updated_at?: string;
}
export interface Allowance {
  account_id: string;
  account: string;
  provider: UsageProvider;
  plan: string | null;
  window: "primary" | "secondary";
  window_minutes: number;
  used_percent: number;
  window_start: number;
  /** Null when the window has run out and the next has not begun. */
  resets_at: number | null;
  used_tokens: number;
  used_cost: number;
  estimated_total_tokens: number | null;
  estimated_remaining_tokens: number | null;
  estimated_total_cost: number | null;
  estimated_remaining_cost: number | null;
  low_confidence: boolean;
  /** When the client last logged this reading, if it came from its session log rather than an account check. */
  observed_at: number | null;
}

/** 1.2K, 34.5M, 1.1B: tokens at a glance. */
export function formatTokens(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0";
  const units = [[1e9, "B"], [1e6, "M"], [1e3, "K"]] as const;
  for (const [size, unit] of units) if (value >= size) { const scaled = value / size; return `${scaled >= 100 ? Math.round(scaled) : scaled.toFixed(1)}${unit}`; }
  return String(Math.round(value));
}

export function formatCost(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "$0";
  return value >= 100 ? `$${Math.round(value).toLocaleString("en-US")}` : value >= 1 ? `$${value.toFixed(2)}` : `$${value.toFixed(3)}`;
}

/** Change against the previous period, or nothing when there was no previous. */
export function trend(current: number, previous: number): number | undefined {
  if (!(previous > 0)) return undefined;
  return (current - previous) / previous * 100;
}

/** Input the cache served, of all input: cache reads over reads, writes and uncached input. */
export function cacheHitRate(bucket: Pick<UsageBucket, "input" | "cache_read" | "cache_write_5m" | "cache_write_1h">): number {
  const input = bucket.input + bucket.cache_read + bucket.cache_write_5m + bucket.cache_write_1h;
  return input > 0 ? bucket.cache_read / input * 100 : 0;
}

/** What cache reads saved against paying full input price for them, where the price is known. */
export function cacheSavings(models: readonly UsageModel[]): number {
  return models.reduce((sum, row) => sum + (row.price ? row.cache_read * (row.price.input - row.price.cache_read) / 1e6 : 0), 0);
}

/** Output tokens a second, from the request leaving to the reply's last token; undefined when no call was timed. */
export function outputSpeed(bucket: Pick<UsageBucket, "timed_output" | "timed_ms">): number | undefined {
  return bucket.timed_ms && bucket.timed_output !== undefined ? bucket.timed_output / bucket.timed_ms * 1000 : undefined;
}

/** Requests that ended in an error, in percent of those a client logs the outcome of; undefined when none does. */
export function failureRate(bucket: Pick<UsageBucket, "answered" | "failed">): number | undefined {
  const failed = bucket.failed ?? 0, tried = (bucket.answered ?? 0) + failed;
  return tried ? failed / tried * 100 : undefined;
}

/** The local start of today, and of the day N days back, as Unix seconds. */
export function rangeStart(days: number, now = new Date()): number {
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate() - (days - 1));
  return Math.floor(start.getTime() / 1000);
}

/** 9h 50m, 1292h 59m: time spent, in hours past a day as the dashboard reads it. */
export function formatHours(minutes: number): string {
  const total = Math.max(0, Math.round(minutes));
  const hours = Math.floor(total / 60), rest = total % 60;
  return hours ? `${hours}h ${rest}m` : `${rest}m`;
}
