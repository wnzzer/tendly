/**
 * Model checks: the batch of "does this model actually run?" answers the
 * server sends back, and what a person can do with them.
 *
 * A check costs one request per model, so nothing here decides to run one on
 * its own; it only orders them, tallies them, and turns them into the two
 * pruning actions the form offers. Kept apart from the component so the rules
 * (which statuses prove a model is broken, what "keep working only" means)
 * can be tested without a browser.
 */
import type { ModelProbeApi, ModelProbePlan, ModelProbeResult, ModelProbeStatus } from "@tendly/protocol";

/**
 * `ok` ran, `unauthorized` the key was refused (a credential problem, not a
 * model one), `not_found` the endpoint has no such model, `rate_limited` it
 * asked to be left alone, `timeout` it did not answer, `failed` anything else.
 * The server owns this vocabulary (model_probe.rs) and the protocol carries it.
 */
export type ProbeStatus = ModelProbeStatus;
export type { ModelProbeApi, ModelProbePlan, ModelProbeResult };

/**
 * How each wire protocol is named on screen. The same model can answer on one
 * API and fail on another — a gateway may serve `openai-completions` but not
 * `openai-responses` — so a check is only meaningful with the protocol it
 * used, and the panel says which.
 */
export const PROBE_APIS: Readonly<Record<ModelProbeApi, { label: string; short: string }>> = {
  "anthropic-messages": { label: "Anthropic Messages", short: "Messages" },
  "openai-responses": { label: "OpenAI Responses", short: "Responses" },
  "openai-completions": { label: "OpenAI Chat Completions", short: "Chat" },
  "google-generative-ai": { label: "Google Generative AI", short: "Google" },
};

/**
 * The one protocol each client speaks, for the clients that have no choice.
 * Claude Code always speaks Anthropic Messages and Codex always speaks the
 * Responses API, whatever the endpoint; only Pi can be told which to use.
 */
export const FIXED_APIS: Readonly<Record<string, ModelProbeApi>> = {
  claude_code: "anthropic-messages",
  codex: "openai-responses",
};

/** The protocols a batch of answers used, in the order they first appear. */
export function apisUsed(results: readonly ModelProbeResult[]): ModelProbeApi[] {
  const seen: ModelProbeApi[] = [];
  for (const result of results) {
    if (result?.api && !seen.includes(result.api)) seen.push(result.api);
  }
  return seen;
}

/** Results by model ID. A model with no entry has not been checked. */
export type ProbeChecks = Readonly<Record<string, ModelProbeResult>>;

/** Checks per request. The server caps a request at 24; the form asks in
 * smaller batches so progress moves and stopping means stopping soon. */
export const PROBE_BATCH = 8;

/** The models split into the batches to ask for, in order, without repeats. */
export function probeBatches(models: readonly string[], size: number = PROBE_BATCH): string[][] {
  const batch = Math.max(1, Math.floor(size) || 1);
  const unique = [...new Set(models.filter(model => model.trim()))];
  const batches: string[][] = [];
  for (let index = 0; index < unique.length; index += batch) {
    batches.push(unique.slice(index, index + batch));
  }
  return batches;
}

/** Models one press may check. The server caps a request at 24; the panel asks
 * in smaller batches so answers appear as they arrive, so the run itself is
 * capped here too — one press is one bill the person agreed to. */
export const PROBE_RUN = 24;

/**
 * The models one press will check: the first `limit` that still need an
 * answer. What is left over is reported, not silently dropped. `again` is the
 * deliberate re-check of a row that already answered — asking twice is the
 * point there, so nothing is skipped.
 */
export function probeRun(
  models: readonly string[],
  checks: ProbeChecks,
  limit: number = PROBE_RUN,
  again = false,
): { models: string[]; skipped: number } {
  const unique = [...new Set(models.filter(model => model.trim()))];
  const wanted = again ? unique : unique.filter(model => needsCheck(checks[model]?.status));
  const size = Math.max(1, Math.floor(limit) || 1);
  return { models: wanted.slice(0, size), skipped: Math.max(0, wanted.length - size) };
}

/** The checks after a batch answers; a repeat replaces what was there. */
export function mergeChecks(checks: ProbeChecks, results: readonly ModelProbeResult[]): Record<string, ModelProbeResult> {
  const next: Record<string, ModelProbeResult> = { ...checks };
  for (const result of results) {
    if (result && typeof result.model === "string" && result.model) next[result.model] = result;
  }
  return next;
}

/**
 * Whether a check proved the endpoint cannot run the model. Only a missing
 * model or a refusal does: a rate limit, a timeout and a rejected key say
 * something about the moment or the key, and pruning on those would delete a
 * model that works.
 */
export function isBroken(status: ProbeStatus | undefined): boolean {
  return status === "not_found" || status === "failed";
}

/**
 * Whether a whole batch of 404s is more likely about the route than the models.
 * A catalogue can be out of date about one model; it is not usually wrong
 * about every one of them. A client that speaks a single protocol (Claude Code
 * asks `/v1/messages`, Codex asks `/v1/responses`) gets a 404 for every model
 * when the endpoint has no such route, and that reads as "all the models are
 * gone" unless it is said out loud.
 */
export function routeSuspicion(results: readonly ModelProbeResult[]): boolean {
  const answered = results.filter(result => result.status !== "unauthorized");
  return answered.length > 1 && answered.every(result => result.status === "not_found");
}

/** Whether a check left the model's fate open: worth showing, not worth pruning. */
export function isInconclusive(status: ProbeStatus | undefined): boolean {
  return status === "unauthorized" || status === "rate_limited" || status === "timeout";
}

/** Whether a model still needs a check: no answer, or one that proved nothing. */
export function needsCheck(status: ProbeStatus | undefined): boolean {
  return !status || isInconclusive(status);
}

export interface CheckTally {
  /** Models that have an answer. */
  tested: number;
  ok: number;
  /** Models proven not to run here. */
  broken: number;
  /** Checked, but the answer said nothing about the model itself. */
  inconclusive: number;
}

export function checkTally(checks: ProbeChecks): CheckTally {
  const tally: CheckTally = { tested: 0, ok: 0, broken: 0, inconclusive: 0 };
  for (const result of Object.values(checks)) {
    tally.tested += 1;
    if (result.status === "ok") tally.ok += 1;
    else if (isBroken(result.status)) tally.broken += 1;
    else if (isInconclusive(result.status)) tally.inconclusive += 1;
  }
  return tally;
}

/** The chosen models whose check passed, in the order chosen. */
export function keepWorking(selected: readonly string[], checks: ProbeChecks): string[] {
  return selected.filter(model => checks[model]?.status === "ok");
}

/** The chosen models left after removing the ones proven not to run. */
export function dropBroken(selected: readonly string[], checks: ProbeChecks): string[] {
  return selected.filter(model => !isBroken(checks[model]?.status));
}

/** The chips above the list: everything, the chosen rows, and each outcome. */
export type CheckFilter = "all" | "chosen" | "ok" | "broken" | "inconclusive" | "untested";

export function matchesCheckFilter(
  model: string,
  filter: CheckFilter,
  checks: ProbeChecks,
  chosen: ReadonlySet<string>,
): boolean {
  const status = checks[model]?.status;
  switch (filter) {
    case "chosen":
      return chosen.has(model);
    case "ok":
      return status === "ok";
    case "broken":
      return isBroken(status);
    case "inconclusive":
      return isInconclusive(status);
    case "untested":
      return !status;
    default:
      return true;
  }
}

type Translate = (text: string, values?: Record<string, string | number>) => string;

/** What an answer's pill says, in the panel and in the form's own check. */
export function probeAnswer(result: ModelProbeResult, t: Translate): { kind: "ok" | "broken" | "warn"; text: string; title?: string } {
  const detail = result.detail ? { title: result.detail } : {};
  const latency = formatLatency(result.latency_ms);
  switch (result.status) {
    case "ok": return { kind: "ok", text: latency ? t("Works · {latency}", { latency }) : t("Works"), ...detail };
    case "not_found": return { kind: "broken", text: t("Model not found"), ...detail };
    case "unauthorized": return { kind: "warn", text: t("Key refused"), ...detail };
    case "rate_limited": return { kind: "warn", text: t("Rate limited"), ...detail };
    case "timeout": return { kind: "warn", text: t("Timed out"), ...detail };
    default: return { kind: "broken", text: t("Failed"), ...detail };
  }
}

/**
 * The protocol to keep once one model was tried on each a client can speak:
 * the one the profile already names if it worked, then the one the URL
 * suggests, then the first that worked in the client's own order. None when
 * nothing worked — a refused key or a timeout proves nothing about a protocol,
 * so the profile is left as it was rather than switched on no evidence.
 */
export function chooseApi(
  results: readonly ModelProbeResult[],
  order: readonly ModelProbeApi[],
  named?: ModelProbeApi,
  guessed?: ModelProbeApi,
): ModelProbeApi | undefined {
  const working = new Set(results.filter(result => result.status === "ok").map(result => result.api));
  return [named, guessed, ...order].find((api): api is ModelProbeApi => !!api && working.has(api));
}

/** "820 ms" — one decimal only where it says something. */
export function formatLatency(ms: number | undefined): string {
  if (typeof ms !== "number" || !Number.isFinite(ms) || ms < 0) return "";
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`;
}

/**
 * The header a key refused everywhere is tried in next, over `api`: the
 * other of the two any client sends, from whichever the form uses now (its
 * own preset, or the protocol's: Bearer for the OpenAI APIs, a key header for
 * the rest). A header of the endpoint's own is never swapped for a guess.
 */
export function otherKeyHeader(mode: "" | "x-api-key" | "bearer" | "custom" | undefined, api: ModelProbeApi): "x-api-key" | "bearer" | undefined {
  if (mode === "custom") return undefined;
  const now = mode || (api.startsWith("openai-") ? "bearer" : "x-api-key");
  return now === "bearer" ? "x-api-key" : "bearer";
}
