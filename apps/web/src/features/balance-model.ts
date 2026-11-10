export type BalanceUnit = { kind: 'money'; currency: string } | { kind: 'percentage' };
/** Where the server puts the endpoint key; `{key}` marks the spot and the script never sees the key. */
export type BalanceAuth = { kind: 'none' } | { kind: 'header'; name: string; value: string } | { kind: 'query'; name: string };
export interface BalanceConfig {
  unit: BalanceUnit;
  query_url: string;
  auth: BalanceAuth;
  secret_ref?: string | null;
  script: string;
}
export interface BalanceReading { value: string; unit: BalanceUnit; checked_at: string }
export interface BalanceView {
  config: BalanceConfig | null;
  reading: BalanceReading | null;
  attempted_at: string | null;
  error: string | null;
  stale: boolean;
}
export interface EndpointBalance extends Omit<BalanceView, 'config'> { id: string; name: string }

/** Keep money as decimal text, without floating-point rounding or currency conversion. */
export function balanceLabel(reading: BalanceReading): string {
  return reading.unit.kind === 'money' ? `${reading.unit.currency} ${reading.value}` : `${reading.value}%`;
}
export function balanceIsStale(reading: BalanceReading | null, stale = false, now = Date.now()): boolean {
  return stale || !!reading && now - Date.parse(reading.checked_at) > 300_000;
}
/** Rounds decimal text half away from zero without passing through a float, so large balances stay exact. */
export function roundDecimal(value: string, places: number): string {
  const match = /^(-)?(\d+)(?:\.(\d+))?$/.exec(value.trim());
  if (!match) return value;
  const [, minus, whole, fraction = ''] = match;
  const digits = BigInt(whole + fraction.padEnd(places, '0').slice(0, places)) + ((fraction[places] ?? '0') >= '5' ? 1n : 0n);
  const text = digits.toString().padStart(places + 1, '0');
  const rounded = places ? `${text.slice(0, -places)}.${text.slice(-places)}` : text;
  return minus && digits !== 0n ? `-${rounded}` : rounded;
}

/**
 * The short figure for a chip or menu: "$1,234.57" or "42%"; `compact` drops
 * the cents from 100 up ("$1,235") so a chip leaves room for its name. The
 * full label stays in balanceLabel.
 */
export function balanceShort(reading: BalanceReading, locale?: string, compact = false): string {
  if (reading.unit.kind === 'percentage') {
    const percent = Number(reading.value);
    return `${percent < 10 ? Math.round(percent * 10) / 10 : Math.round(percent)}%`;
  }
  const whole2 = roundDecimal(reading.value, 2), places = compact && /^-?\d{3,}/.test(whole2) ? 0 : 2;
  const rounded = roundDecimal(reading.value, places), negative = rounded.startsWith('-');
  const [whole, fraction] = rounded.replace('-', '').split('.');
  let symbol = reading.unit.currency;
  try { symbol = new Intl.NumberFormat(locale, { style: 'currency', currency: symbol, currencyDisplay: 'narrowSymbol' }).formatToParts(0).find(part => part.type === 'currency')?.value ?? symbol; } catch { /* An unknown code stays as typed. */ }
  const spaced = /^[A-Za-z]{2,}$/.test(symbol) ? `${symbol} ` : symbol;
  return `${negative ? '-' : ''}${spaced}${BigInt(whole).toLocaleString(locale)}${fraction ? `.${fraction}` : ''}`;
}

/** Colour for a reading: a nearly spent percentage, or money at or below zero. Money has no other threshold to judge by. */
export function balanceLevel(reading: BalanceReading): 'ok' | 'warn' | 'danger' {
  if (reading.unit.kind === 'percentage') {
    const percent = Number(reading.value);
    return percent <= 10 ? 'danger' : percent <= 25 ? 'warn' : 'ok';
  }
  return reading.value.trim().startsWith('-') || !/[1-9]/.test(reading.value) ? 'danger' : 'ok';
}
/**
 * Whether to ask the provider again without a click: the last reading is over
 * a minute old (or missing) and this page has not asked within the minute.
 */
export function shouldAutoRefresh(row: Pick<EndpointBalance, 'reading'>, lastAttempt: number | undefined, now: number): boolean {
  if (lastAttempt !== undefined && now - lastAttempt < 60_000) return false;
  return !row.reading || now - Date.parse(row.reading.checked_at) > 60_000;
}
/** The authentication menu: two common headers by name, then any header, any query parameter, or none. */
export type AuthPreset = 'none' | 'bearer' | 'api_key' | 'header' | 'query';
export interface AuthFields { preset: AuthPreset; headerName: string; headerValue: string; queryName: string }
const BEARER = { name: 'Authorization', value: 'Bearer {key}' }, API_KEY = { name: 'x-api-key', value: '{key}' };

export function authFields(auth: BalanceAuth): AuthFields {
  const blank = { headerName: 'X-Token', headerValue: '{key}', queryName: 'api_key' };
  if (auth.kind === 'query') return { ...blank, preset: 'query', queryName: auth.name };
  if (auth.kind === 'none') return { ...blank, preset: 'none' };
  const same = (preset: { name: string; value: string }) => auth.name.toLowerCase() === preset.name.toLowerCase() && auth.value === preset.value;
  return { ...blank, preset: same(BEARER) ? 'bearer' : same(API_KEY) ? 'api_key' : 'header', headerName: auth.name, headerValue: auth.value };
}
export function authFromFields(fields: AuthFields): BalanceAuth {
  switch (fields.preset) {
    case 'none': return { kind: 'none' };
    case 'bearer': return { kind: 'header', ...BEARER };
    case 'api_key': return { kind: 'header', ...API_KEY };
    case 'query': return { kind: 'query', name: fields.queryName.trim() };
    default: return { kind: 'header', name: fields.headerName.trim(), value: fields.headerValue };
  }
}
export const BALANCE_EXAMPLE = 'const data = await ctx.request();\nreturn { value: data.balance };';

/** Starting points offered beside the script editor; each matches an example in docs/custom-balances.md. */
export const BALANCE_EXAMPLES: readonly { id: string; label: string; script: string }[] = [
  { id: 'field', label: 'Read a balance field', script: BALANCE_EXAMPLE },
  { id: 'cents', label: 'Convert cents exactly', script: "// The API reports cents; return exact decimal text, no float rounding.\nconst data = await ctx.request();\nconst cents = BigInt(data.balance_cents);\nconst absolute = cents < 0n ? -cents : cents;\nreturn { value: `${cents < 0n ? '-' : ''}${absolute / 100n}.${String(absolute % 100n).padStart(2, '0')}` };" },
  { id: 'used', label: 'Remaining from used percentage', script: '// The endpoint reports the percentage used.\nconst data = await ctx.request();\nreturn { value: 100 - data.used_percent };' },
  { id: 'quota', label: 'Remaining from total and left', script: "const data = await ctx.request();\nif (!(data.total > 0)) throw new Error('No usable quota total');\nreturn { value: (100 * data.remaining / data.total).toFixed(2) };" },
  { id: 'post', label: 'POST to another path', script: "const data = await ctx.request({\n  url: '/account/balance', // same origin as the query URL\n  method: 'POST',\n  body: { account: 'example' },\n});\nreturn { value: data.available };" },
];

/**
 * Asking again on a timer while a page is open, so a figure left on screen
 * keeps up without a click. Custom balances are a request to an endpoint you
 * configured, and refresh by default; an official allowance is checked against
 * the provider's own rationed endpoint, so it waits until it is switched on,
 * and less often.
 */
export const BALANCE_AUTO_REFRESH_MS = 5 * 60_000;
export const OFFICIAL_AUTO_REFRESH_MS = 15 * 60_000;
export interface AutoRefreshPreferences { balance_auto_refresh?: boolean; official_auto_refresh?: boolean }
export const balanceAutoRefresh = (preferences: AutoRefreshPreferences) => preferences.balance_auto_refresh !== false;
export const officialAutoRefresh = (preferences: AutoRefreshPreferences) => preferences.official_auto_refresh === true;

/** A custom balance whose reading, and this page's last try, are both a full interval old. */
export function balanceDue(row: Pick<EndpointBalance, 'reading'>, lastAttempt: number | undefined, now: number): boolean {
  if (lastAttempt !== undefined && now - lastAttempt < BALANCE_AUTO_REFRESH_MS) return false;
  return !row.reading || now - Date.parse(row.reading.checked_at) >= BALANCE_AUTO_REFRESH_MS;
}

/** How an official account's allowance is asked for: Claude's rationed usage check, or Codex's account refresh, which reports it for free. */
export function officialQuotaSource(account: { capabilities: { usage?: boolean; quota: boolean; refresh_token: boolean } }): 'usage' | 'refresh' | undefined {
  return account.capabilities.usage ? 'usage' : account.capabilities.quota || account.capabilities.refresh_token ? 'refresh' : undefined;
}

/** A signed-in official account last checked a full interval ago, outside any wait the provider asked for. */
export function officialDue(account: { status: string; checked_at?: string; capabilities: { usage?: boolean; quota: boolean; refresh_token: boolean } }, lastAttempt: number | undefined, notBefore: number | undefined, now: number): boolean {
  if (account.status !== 'signed_in' || !officialQuotaSource(account)) return false;
  if (notBefore !== undefined && now < notBefore) return false;
  if (lastAttempt !== undefined && now - lastAttempt < OFFICIAL_AUTO_REFRESH_MS) return false;
  const checked = account.checked_at ? Date.parse(account.checked_at) : NaN;
  return !Number.isFinite(checked) || now - checked >= OFFICIAL_AUTO_REFRESH_MS;
}
