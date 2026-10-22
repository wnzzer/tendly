import type { EndpointProfile, Session } from '@tendly/protocol';
import type { Attachment } from './attachment-state';

/** A model exactly as the native client described it. `efforts` lists the
 * thinking depths that model supports; a model that states none supports none,
 * and no default ladder is substituted. */
/** `isDefault` marks the model a configuration resolves to when none is named,
 * which is what a not-yet-started session shows instead of guessing. */
export interface NativeModel { id: string; name: string; description?: string; efforts?: string[]; isDefault?: boolean }
/** What a native request reaches for, as the bridge describes it. */
export interface ApprovalScope { access?: 'read' | 'write' | 'execute' | 'network'; outside?: string[]; command?: string }
export interface ApprovalQuestion { id: string; header?: string; question: string; options: Array<{ label: string; description?: string }>; isSecret?: boolean; isOther?: boolean; multiSelect?: boolean }
/** What a turn cost, as the client summed it: every token read (`cached` of
 * them from the prompt cache) and every token written. */
export interface TurnUsage { input?: number; output?: number; cached?: number }
/** What the client counted for a background task: tokens, tool calls, run time. */
export interface ToolStats { tokens?: number; tools?: number; ms?: number }
export type ChatEvent = ({ seq?: number } & (
  | { type: 'ready'; native_session_id?: string; commands?: string[] }
  | { type: 'settings'; model?: string; effort?: string; models?: NativeModel[]; permission_mode?: string; permission_modes?: string[] }
  | { type: 'message'; id: string; role: 'user' | 'assistant'; text: string; delta?: boolean; steered?: boolean; provider?: string }
  | { type: 'tool'; id: string; name: string; status: 'running' | 'completed' | 'failed'; text?: string; activity?: string; parent?: string; stats?: ToolStats; at?: number; background?: boolean; result_tokens?: number }
  | { type: 'approval'; id: string; title: string; text: string; choices: string[]; questions?: ApprovalQuestion[]; scope?: ApprovalScope }
  | { type: 'approval_resolved'; id: string }
  | { type: 'turn'; id?: string; status: 'running' | 'completed' | 'failed' | 'interrupted' }
  | { type: 'usage'; input_tokens?: number; output_tokens?: number; context_tokens?: number; context_window?: number; turn?: TurnUsage; message_id?: string }
  | { type: 'configuration'; id: string; profile_name?: string; text: string }
  | { type: 'error'; message: string }
  | { type: 'exit' }
  | { type: 'cleared' }
));
/** `after` marks a resumed greeting: `events` holds only what follows that sequence and adds to the view. */
export interface ConversationSnapshot { mode?: 'structured' | 'pty'; running: boolean; events: ChatEvent[]; truncated?: boolean; after?: number; older?: number }
export type ChatItem =
  /** `provider`: the client that wrote a reply carried over from another session. */
  | { type: 'message'; id: string; role: 'user' | 'assistant'; text: string; steered?: boolean; usage?: TurnUsage; provider?: string }
  | ToolItem
  | { type: 'approval'; id: string; title: string; text: string; choices: string[]; questions: ApprovalQuestion[]; scope?: ApprovalScope; resolved: boolean }
  | { type: 'configuration'; id: string; profile_name?: string; text: string }
  | { type: 'error'; id: string; text: string };

/** A tool call; `children` are the calls a subagent made inside it, nested as deep as they went. */
export interface ToolItem { type: 'tool'; id: string; name: string; status: 'running' | 'completed' | 'failed'; text: string; activity?: string; parent?: string; stats?: ToolStats; at?: number; ms_at?: number; since?: number; label?: string; ended?: boolean; background?: boolean; result_tokens?: number; children?: ToolItem[] }

/** The tools that start a subagent. Claude Code renamed Task to Agent. */
export const SUBAGENT_TOOLS = new Set(['Agent', 'Task']);
/**
 * A few words on what a call does, from its input: what the agent said it was
 * for, else the command, file, pattern or query it names. A bare "Bash" said
 * nothing. Read only while the call runs, since its result replaces the input.
 */
export function toolLabel(input: string | undefined) {
  if (!input?.startsWith('{')) return;
  let value: Record<string, unknown>;
  try { value = JSON.parse(input) as Record<string, unknown>; } catch { return; }
  const text = (key: string) => typeof value[key] === 'string' && (value[key] as string).trim() ? (value[key] as string).trim() : undefined;
  const short = (line: string) => { const first = line.split('\n')[0]!; return first.length > 80 ? `${first.slice(0, 79)}…` : first; };
  // The last two segments say which file without spelling out the whole path.
  const file = (path: string) => path.split(/[\\/]/).filter(Boolean).slice(-2).join('/');
  const described = text('description'), command = text('command'), path = text('file_path') ?? text('notebook_path') ?? text('path');
  const pattern = text('pattern'), query = text('query'), url = text('url');
  if (described) return short(described);
  if (command) return short(command);
  if (pattern) return short(path ? `${pattern} · ${file(path)}` : pattern);
  if (path) return file(path);
  if (query) return short(query);
  if (url) return short(url.replace(/^https?:\/\//, ''));
}

/** Newest-first truncation: the latest subagent step matters more than its first. */
const MAX_ACTIVITY = 4000;
function clipTail(value: string, limit: number) {
  return value.length <= limit ? value : `…\n${value.slice(value.length - limit)}`;
}

/**
 * What the conversation shows, rebuilt from its event log.
 *
 * Events are folded in order, each sequence number once: messages and their
 * streamed deltas, tool calls (those a subagent made are hung under it at the
 * end), approvals and their resolution, the turn's state, usage, the
 * commands, models and permission modes the client announced, and its exit.
 * The result is everything the pane renders from: the items, the turn, usage
 * and the client's settings. `running: false` says the process is gone, so a
 * turn still running reads as interrupted and no approval is left waiting.
 */
export function conversationView(events: readonly ChatEvent[], running?: boolean) {
  const items: ChatItem[] = [], indexes = new Map<string, number>(), seen = new Set<number>();
  // A call still running when its client exits will never hear back: that
  // process, and any subagent it ran, is gone. It is marked ended rather than
  // left to read as running for ever after the client restarts.
  const endRunning = () => {
    for (let i = 0; i < items.length; i++) { const item = items[i]; if (item.type === 'tool' && item.status === 'running') items[i] = { ...item, ended: true }; }
    for (const [id, tool] of nested) if (tool.status === 'running') nested.set(id, { ...tool, ended: true });
  };
  // Calls made inside a subagent live here rather than in `items`, and are hung
  // under their parent once every event has been read.
  const nested = new Map<string, ToolItem>(), childrenOf = new Map<string, string[]>();
  let turn: 'idle' | 'running' | 'completed' | 'failed' | 'interrupted' = 'idle';
  /** Where the current turn's items begin, so its cost finds its own reply. */
  let turnStart = 0;
  let ready = false, exited = false, commands: string[] = [], announced = false, models: NativeModel[] = [], model: string | undefined, effort: string | undefined, permissionMode: string | undefined, permissionModes: string[] = [], usage: { input_tokens?: number; output_tokens?: number; context_tokens?: number; context_window?: number } = {};
  for (const event of events) {
    if (event.seq !== undefined) { if (seen.has(event.seq)) continue; seen.add(event.seq); }
    if (event.type === 'message') {
      const key = 'message:' + event.id, index = indexes.get(key), previous = index === undefined ? undefined : items[index];
      const steered = event.steered === true || (previous?.type === 'message' && previous.steered === true);
      const kept = previous?.type === 'message' ? previous.usage : undefined;
      const next: ChatItem = { type: 'message', id: event.id, role: event.role, text: event.delta && previous?.type === 'message' ? previous.text + event.text : event.text, ...(steered ? { steered: true } : {}), ...(kept ? { usage: kept } : {}), ...(event.provider ? { provider: event.provider } : {}) };
      if (index === undefined) { indexes.set(key, items.length); items.push(next); } else items[index] = next;
    } else if (event.type === 'tool') {
      const key = 'tool:' + event.id, index = indexes.get(key), previous = index === undefined ? nested.get(event.id) : items[index];
      // Subagent progress accumulates so the card reads as a log, while `text`
      // still carries only the tool's own input and result. Capped because a
      // long-running subagent would otherwise grow this without bound.
      const before = previous?.type === 'tool' ? previous : undefined;
      const activity = event.activity === undefined ? before?.activity
        : clipTail(before?.activity ? `${before.activity}\n${event.activity}` : event.activity, MAX_ACTIVITY);
      const parent = event.parent ?? before?.parent, stats = event.stats ?? before?.stats, at = event.at ?? before?.at, background = event.background ?? before?.background, resultTokens = event.result_tokens ?? before?.result_tokens;
      // When the reported duration was measured, and when the client first said
      // anything timed: a running card counts on from these between reports.
      const msAt = event.stats?.ms !== undefined && event.at !== undefined ? event.at : before?.ms_at, since = before?.since ?? event.at;
      const label = before?.label ?? (event.status === 'running' ? toolLabel(event.text) : undefined);
      const next: ToolItem = { ...event, text: event.text ?? before?.text ?? '', ...(activity === undefined ? {} : { activity }), ...(parent === undefined ? {} : { parent }), ...(stats === undefined ? {} : { stats }), ...(at === undefined ? {} : { at }), ...(msAt === undefined ? {} : { ms_at: msAt }), ...(since === undefined ? {} : { since }), ...(label === undefined ? {} : { label }), ...(background ? { background } : {}), ...(resultTokens === undefined ? {} : { result_tokens: resultTokens }) };
      // A parent trimmed out of history leaves its calls at the top rather than nowhere.
      if (parent !== undefined && index === undefined && parent !== event.id && (indexes.has('tool:' + parent) || nested.has(parent))) {
        if (!nested.has(event.id)) childrenOf.set(parent, [...(childrenOf.get(parent) ?? []), event.id]);
        nested.set(event.id, next);
      } else if (index === undefined) { indexes.set(key, items.length); items.push(next); } else items[index] = next;
    } else if (event.type === 'approval') {
      const key = 'approval:' + event.id, index = indexes.get(key);
      const next: ChatItem = { ...event, questions: event.questions ?? [], resolved: false };
      if (index === undefined) { indexes.set(key, items.length); items.push(next); } else items[index] = next;
    } else if (event.type === 'approval_resolved') {
      const index = indexes.get('approval:' + event.id), previous = index === undefined ? undefined : items[index];
      if (previous?.type === 'approval') previous.resolved = true;
    } else if (event.type === 'turn') {
      // A steer the last turn ended without reading runs as a turn of its own,
      // under its id: it moves to open that turn, no longer marked as steered.
      const key = 'message:' + event.id, at = event.status === 'running' && event.id ? indexes.get(key) : undefined, held = at === undefined ? undefined : items[at];
      if (at !== undefined && held?.type === 'message' && held.steered) {
        items.splice(at, 1);
        for (const [other, index] of indexes) if (index > at) indexes.set(other, index - 1);
        const { steered: _steered, ...unsteered } = held;
        indexes.set(key, items.length); items.push(unsteered);
      }
      if (event.status === 'running') turnStart = items.length;
      turn = event.status;
    }
    else if (event.type === 'ready') { ready = true; exited = false; commands = event.commands ?? []; announced = true; }
    else if (event.type === 'settings') { if (event.models?.length) models = event.models; if (event.model) model = event.model; if (event.effort) effort = event.effort; if (event.permission_mode) permissionMode = event.permission_mode; if (event.permission_modes?.length) permissionModes = event.permission_modes; }
    // A turn's cost sits under that turn's last reply; the rest is the live context.
    else if (event.type === 'usage' && event.turn) {
      const index = event.message_id ? indexes.get('message:' + event.message_id) : undefined;
      if (event.message_id) {
        const item = index === undefined ? undefined : items[index];
        if (index !== undefined && item?.type === 'message' && item.role === 'assistant') items[index] = { ...item, usage: event.turn };
      } else for (let i = items.length - 1; i >= turnStart; i--) { const item = items[i]; if (item.type === 'message' && item.role === 'assistant') { items[i] = { ...item, usage: event.turn }; break; } }
    }
    else if (event.type === 'usage') { const { seq: _seq, type: _type, turn: _turn, ...values } = event; usage = { ...usage, ...values }; }
    // A new endpoint means a new native client: its command list is whatever it
    // announces next, never the previous client's.
    else if (event.type === 'configuration') { endRunning(); for (const item of items) if (item.type === 'approval') item.resolved = true; indexes.clear(); nested.clear(); childrenOf.clear(); items.push(event); ready = false; turn = 'idle'; commands = []; announced = false; models = []; model = undefined; effort = undefined; permissionMode = undefined; permissionModes = []; }
    // The client cleared its own context. Keeping the transcript would show a
    // conversation the client can no longer refer to, which is what made
    // /clear look like it had done nothing.
    else if (event.type === 'cleared') { items.length = 0; turnStart = 0; indexes.clear(); nested.clear(); childrenOf.clear(); usage = {}; }
    else if (event.type === 'error') items.push({ type: 'error', id: 'error:' + (event.seq ?? items.length), text: event.message });
    else if (event.type === 'exit') { endRunning(); exited = true; ready = false; if (turn === 'running') turn = 'interrupted'; for (const item of items) if (item.type === 'approval') item.resolved = true; }
  }
  // Runtime state outranks stale display history after server restart. Old
  // approvals refer to lost native RPCs and cannot be answered as live ones.
  const hang = (item: ToolItem, depth: number): ToolItem => {
    const ids = depth < 16 ? childrenOf.get(item.id) : undefined;
    return ids ? { ...item, children: ids.map(id => hang(nested.get(id)!, depth + 1)) } : item;
  };
  for (let i = 0; i < items.length; i++) { const item = items[i]; if (item.type === 'tool' && childrenOf.has(item.id)) items[i] = hang(item, 0); }
  if (running === false) { ready = false; if (turn === 'running') turn = 'interrupted'; for (const item of items) if (item.type === 'approval') item.resolved = true; }
  return { items, turn, ready, exited, usage, commands, commandsAnnounced: announced, models, model, effort, permissionMode, permissionModes, awaitingApproval: items.some(item => item.type === 'approval' && !item.resolved) };
}

/** WS events are already ordered. A replayed sequence must not append deltas twice. */
export function appendChatEvent(events: readonly ChatEvent[], event: ChatEvent): ChatEvent[] {
  if (event.seq !== undefined && events.some(existing => existing.seq === event.seq)) return [...events];
  // A finished message keeps its place but not its pieces, as the server
  // stores it: the first delta stays, emptied, where the message began among
  // the calls made while it streamed; the final message carries the text.
  if (event.type === 'message' && !event.delta) {
    let first = -1;
    const kept = events.filter((existing, index) => {
      if (existing.type !== 'message' || !existing.delta || existing.id !== event.id) return true;
      if (first === -1) { first = index; return true; }
      return false;
    });
    if (first !== -1) {
      const place = events[first]!;
      kept[kept.indexOf(place)] = { ...place, text: '' } as ChatEvent;
      return [...kept, event];
    }
  }
  return [...events, event];
}
/** Each event's size on the wire, measured once: events never change. */
const eventSizes = new WeakMap<ChatEvent, number>();
const encoder = new TextEncoder();
function eventBytes(event: ChatEvent): number {
  let bytes = eventSizes.get(event);
  if (bytes === undefined) { bytes = encoder.encode(JSON.stringify(event)).byteLength; eventSizes.set(event, bytes); }
  return bytes;
}
/**
 * What a view keeps in memory: history is kept in full on the server and read
 * in pages, so this bounds only how much one view holds (older pages read again
 * when scrolled back to), not what the session remembers.
 */
export function pruneChatEvents(events: readonly ChatEvent[], maxHistoryEvents = 20000, maxHistoryBytes = 32 * 1024 * 1024) {
  const controls = new Map<string, ChatEvent>(), approvals = new Map<string, ChatEvent>();
  const size = eventBytes;
  for (const event of events) {
    // The model list and the current selection can arrive in separate settings
    // events, so the last of each is kept rather than only the last overall.
    // A turn's cost is history like its reply; only the live context is a control.
    if (['ready', 'turn', 'exit', 'configuration', 'usage', 'settings', 'cleared'].includes(event.type) && !(event.type === 'usage' && event.turn)) {
      controls.set(event.type === 'settings' && event.models?.length ? 'settings:models' : event.type, event);
    }
    if (event.type === 'approval') approvals.set(event.id, event);
    if (event.type === 'approval_resolved') approvals.delete(event.id);
    if (event.type === 'configuration' || event.type === 'exit' || event.type === 'cleared') approvals.clear();
  }
  if (approvals.size > 32) throw Error('Too many unresolved native approvals. Reconnect the view before continuing.');
  const anchors = new Set([...controls.values(), ...approvals.values()]);
  let anchorBytes = 0;
  for (const event of anchors) { const bytes = size(event); if (bytes > 192 * 1024) throw Error('A native control event exceeds the supported display limit.'); anchorBytes += bytes; }
  const keep = new Set(anchors); let historyBytes = 0, historyCount = 0;
  for (let i = events.length - 1; i >= 0; i--) {
    const event = events[i]; if (anchors.has(event)) continue;
    const bytes = size(event);
    if (historyCount >= maxHistoryEvents || historyBytes + bytes > maxHistoryBytes) break;
    keep.add(event); historyBytes += bytes; historyCount++;
  }
  // Filtering the original ordered stream preserves ready/exit/configuration
  // boundaries; protected old approvals must never be appended out of order.
  // The last event let go: a view resumes reading older history after it.
  let dropped: number | undefined;
  for (const event of events) if (!keep.has(event) && event.seq !== undefined && (dropped === undefined || event.seq > dropped)) dropped = event.seq;
  return { events: events.filter(event => keep.has(event)), truncated: keep.size < events.length, historyBytes, anchorBytes, ...(dropped === undefined ? {} : { dropped }) };
}
export function isChatEvent(value: unknown): value is ChatEvent {
  if (!value || typeof value !== 'object') return false;
  const v = value as Record<string, unknown>, text = (key: string) => typeof v[key] === 'string';
  if (v.seq !== undefined && (typeof v.seq !== 'number' || !Number.isSafeInteger(v.seq) || v.seq < 0)) return false;
  switch (v.type) {
    case 'ready': return (v.native_session_id === undefined || text('native_session_id'))
      && (v.commands === undefined || Array.isArray(v.commands) && v.commands.length <= 400 && v.commands.every(name => typeof name === 'string' && /^[A-Za-z0-9][\w:-]{0,63}$/.test(name)));
    case 'settings': return (v.model === undefined || text('model')) && (v.effort === undefined || text('effort'))
      && (v.permission_mode === undefined || text('permission_mode'))
      && (v.permission_modes === undefined || Array.isArray(v.permission_modes) && v.permission_modes.length <= 8 && v.permission_modes.every(mode => typeof mode === 'string' && /^[a-z_]{1,24}$/.test(mode)))
      && (v.models === undefined || Array.isArray(v.models) && v.models.length <= 64 && v.models.every(entry => {
        if (!entry || typeof entry !== 'object') return false;
        const m = entry as Record<string, unknown>;
        return typeof m.id === 'string' && !!m.id && typeof m.name === 'string'
          && (m.description === undefined || typeof m.description === 'string')
          && (m.efforts === undefined || Array.isArray(m.efforts) && m.efforts.every(level => typeof level === 'string'));
      }));
    case 'exit': case 'cleared': return true;
    case 'message': return text('id') && text('text') && ['user', 'assistant'].includes(String(v.role)) && (v.delta === undefined || typeof v.delta === 'boolean') && (v.provider === undefined || text('provider'));
    case 'tool': return text('id') && text('name') && ['running', 'completed', 'failed'].includes(String(v.status)) && (v.text === undefined || text('text')) && (v.activity === undefined || text('activity'))
      && (v.parent === undefined || text('parent') && !!v.parent) && (v.background === undefined || typeof v.background === 'boolean') && (v.result_tokens === undefined || typeof v.result_tokens === 'number' && v.result_tokens >= 0) && (v.at === undefined || typeof v.at === 'number' && Number.isFinite(v.at))
      && (v.stats === undefined || !!v.stats && typeof v.stats === 'object' && Object.entries(v.stats).every(([key, value]) => ['tokens', 'tools', 'ms'].includes(key) && typeof value === 'number' && value >= 0));
    case 'approval': return text('id') && text('title') && text('text') && Array.isArray(v.choices) && v.choices.every(choice => typeof choice === 'string') && (v.questions === undefined || Array.isArray(v.questions) && v.questions.every(question => {
      if (!question || typeof question !== 'object') return false;
      const q = question as Record<string, unknown>;
      return typeof q.id === 'string' && typeof q.question === 'string' && ['isSecret', 'isOther', 'multiSelect'].every(key => q[key] === undefined || typeof q[key] === 'boolean') && Array.isArray(q.options) && q.options.every(option => !!option && typeof option === 'object' && typeof (option as Record<string, unknown>).label === 'string');
    }));
    case 'approval_resolved': return text('id');
    case 'turn': return ['running', 'completed', 'failed', 'interrupted'].includes(String(v.status));
    case 'usage': return ['input_tokens', 'output_tokens', 'context_tokens', 'context_window'].every(key => v[key] === undefined || typeof v[key] === 'number' && Number.isFinite(v[key]) && Number(v[key]) >= 0)
      && (v.message_id === undefined || typeof v.message_id === 'string' && !!v.message_id)
      && (v.turn === undefined || !!v.turn && typeof v.turn === 'object' && Object.entries(v.turn).every(([key, value]) => ['input', 'output', 'cached'].includes(key) && typeof value === 'number' && value >= 0));
    case 'error': return text('message');
    // Earlier cross-client handoffs stored their boundary without a name; those are still read.
    case 'configuration': return text('id') && (v.profile_name === undefined || text('profile_name')) && text('text');
    default: return false;
  }
}
export function parseConversationSnapshot(value: unknown): ConversationSnapshot {
  if (!value || typeof value !== 'object') throw Error('Invalid conversation response.');
  const v = value as Record<string, unknown>;
  if (typeof v.running !== 'boolean' || !Array.isArray(v.events) || !v.events.every(isChatEvent) || (v.mode !== undefined && v.mode !== 'structured' && v.mode !== 'pty')) throw Error('Invalid conversation response.');
  if (v.after !== undefined && !(typeof v.after === 'number' && Number.isSafeInteger(v.after) && v.after >= 0)) throw Error('Invalid conversation response.');
  if (v.older !== undefined && v.older !== null && !(typeof v.older === 'number' && Number.isSafeInteger(v.older) && v.older > 0)) throw Error('Invalid conversation response.');
  return { mode: v.mode as ConversationSnapshot['mode'], running: v.running, events: v.events, truncated: v.truncated === true, ...(v.after !== undefined ? { after: v.after as number } : {}), ...(typeof v.older === 'number' ? { older: v.older } : {}) };
}
/** An older page of a conversation: its events, and the cursor for the page before it. */
export function parseConversationPage(value: unknown): { events: ChatEvent[]; older?: number } {
  if (!value || typeof value !== 'object') throw Error('Invalid conversation response.');
  const v = value as Record<string, unknown>;
  if (v.type !== 'page' || !Array.isArray(v.events) || !v.events.every(isChatEvent)) throw Error('Invalid conversation response.');
  if (v.older !== undefined && v.older !== null && !(typeof v.older === 'number' && Number.isSafeInteger(v.older) && v.older > 0)) throw Error('Invalid conversation response.');
  return { events: v.events, ...(typeof v.older === 'number' ? { older: v.older } : {}) };
}
/** Older events put in front of what a view holds, once each. */
export function prependChatEvents(older: readonly ChatEvent[], events: readonly ChatEvent[]): ChatEvent[] {
  const held = new Set(events.map(event => event.seq).filter(seq => seq !== undefined));
  return [...older.filter(event => event.seq === undefined || !held.has(event.seq)), ...events];
}
/** The last sequence a view holds, where a socket resumes; none for a view that holds nothing yet. */
export function lastChatSeq(events: readonly ChatEvent[]): number | undefined {
  let last: number | undefined;
  for (const event of events) if (event.seq !== undefined && (last === undefined || event.seq > last)) last = event.seq;
  return last;
}

export function approvalPayloadAnswers(questions: readonly ApprovalQuestion[], selections: Record<string, string[]> = {}, others: Record<string, string> = {}): Record<string, string[]> {
  return Object.fromEntries(questions.flatMap(question => {
    const raw = Object.hasOwn(selections, question.id) ? selections[question.id] : [];
    let values = raw.filter(value => typeof value === 'string' && value.trim() && (question.isSecret || question.isOther || !question.options.length || question.options.some(option => option.label === value)));
    const other = question.isOther && Object.hasOwn(others, question.id) ? others[question.id] : undefined;
    if (other?.trim()) values = question.multiSelect && !question.isSecret ? [...values, other] : [other];
    values = [...new Set(values)];
    if (!question.multiSelect || question.isSecret) values = values.slice(0, 1);
    return values.length ? [[question.id, values]] : [];
  }));
}

/**
 * Whether the client's own /clear can be offered right now.
 *
 * This is a pure function because the condition is easy to get subtly wrong:
 * it once included "a message could be sent", which requires typed text, so
 * the action disappeared whenever the composer was empty — exactly when it is
 * wanted. Clearing needs a live, idle client that advertised the command, and
 * nothing about what is in the composer.
 */
export function canClearContext(state: {
  preview: boolean;
  running: boolean;
  ready: boolean;
  busy: boolean;
  connected: boolean;
  commands: readonly string[];
}): boolean {
  return !state.preview && state.running && state.ready && !state.busy && state.connected && state.commands.includes('clear');
}

export interface ChatDraft {
  text: string;
  /** Files attached and not yet sent; kept here, with the text, past a tab switch. */
  attachments?: Attachment[];
  pending?: {
    id: string;
    /** What went on the wire: trimmed, and with any attachment header. */
    content: string;
    /** What the composer held when it was sent. The wire content is not the
     * same string — it is trimmed and may carry an attachment header — so
     * comparing against it left the composer holding a message already sent. */
    text: string;
    state: 'sending' | 'unknown';
    profileId?: string | null;
    configurationRevision?: number;
  };
  notice?: string;
}
export function createChatDraftStore() {
  const drafts = new Map<string, ChatDraft>();
  return { get(id: string): ChatDraft { let draft = drafts.get(id); if (!draft) { draft = { text: '' }; drafts.set(id, draft); } return draft; } };
}
export const chatDrafts = createChatDraftStore();
export function acknowledgeDraft(draft: ChatDraft, events: readonly ChatEvent[]): boolean {
  const pending = draft.pending;
  if (!pending || !events.some(event => event.type === 'message' && event.role === 'user' && event.id === pending.id)) return false;
  // Only the message that was sent is cleared; anything typed since stays.
  if (draft.text === pending.text) draft.text = '';
  draft.pending = undefined; draft.notice = undefined; return true;
}
export function acknowledgeReceipt(draft: ChatDraft, receipt: unknown): boolean {
  const pending=draft.pending;
  if(!pending || !receipt || typeof receipt!=='object' || (receipt as {accepted?:unknown}).accepted!==true || (receipt as {id?:unknown}).id!==pending.id)return false;
  if(draft.text===pending.text)draft.text='';
  draft.pending=undefined;draft.notice=undefined;return true;
}
export function pendingMessageRetry(draft: ChatDraft, session: Pick<Session, 'endpoint_profile_id' | 'configuration_revision'>) {
  const pending = draft.pending;
  if (!pending || pending.state !== 'unknown') throw Error('There is no uncertain message to retry.');
  if ((pending.profileId ?? null) !== (session.endpoint_profile_id ?? null) || (pending.configurationRevision??0)!==(session.configuration_revision??0)) throw Error('The endpoint changed while delivery was uncertain. This request will not be retried with another account.');
  return { id: pending.id, content: pending.content };
}
export function sessionConfigurationPayload(
  session: Session, profiles: EndpointProfile[], profileId: string | null, confirmed: boolean,
  context?: { mode?: string; ready: boolean; busy: boolean; awaitingApproval: boolean },
  /** Session-only overrides; omitted fields keep whatever the profile provides. */
  overrides?: { model?: string; effort?: string },
) {
  if (!confirmed) throw Error('Confirm the endpoint change first.');
  if (!['stopped', 'failed'].includes(session.status) && !(context?.mode === 'structured' && context.ready && !context.busy && !context.awaitingApproval)) throw Error('Wait for the current turn and approvals before changing endpoints.');
  if (profileId !== null && !profiles.some(profile => profile.id === profileId && profile.provider === session.provider)) throw Error('Choose an endpoint for the same client.');
  const model = overrides?.model?.trim();
  const effort = overrides?.effort?.trim();
  return {
    endpoint_profile_id: profileId,
    ...(model ? { model } : {}),
    ...(effort ? { effort } : {}),
    confirmed: true,
  };
}
