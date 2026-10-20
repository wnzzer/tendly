<script setup lang="ts">
import { computed, h, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch, type Ref } from 'vue';
import type { EndpointProfile, Session, Workspace } from '@tendly/protocol';
import { assetUrl, errorMessage, json, providerLabel, request, sessionPath, workspacePath } from './api';
import { backendCapabilities } from './backend-capabilities';
import { agentClient } from './clients';
import { sessionConnections } from './session-connection';
import { createSessionStream, type SessionStreamState } from './session-stream';
import { onPageReturn } from './page-return';
import { acknowledgeDraft, appendChatEvent, approvalPayloadAnswers, canClearContext, chatDrafts, createChatDraftStore, conversationView, isChatEvent, parseConversationSnapshot, pendingMessageRetry, pruneChatEvents, sessionConfigurationPayload, type ChatDraft, type ChatEvent, type ChatItem, type ConversationSnapshot, type NativeModel, type ToolItem, type ToolStats, type TurnUsage, SUBAGENT_TOOLS } from './chat-model';
import ProviderIcon from './ProviderIcon.vue';
import Icon from './Icon.vue';
import ContextMenu from './ContextMenu.vue';
import ChipMenu from './ChipMenu.vue';
import BottomSheet from './BottomSheet.vue';
import AgentToolsSwitch from './AgentToolsSwitch.vue';
import { useMobile } from './mobile';
import { describeModel, resolvedDefaultName } from './model-description';
import ContextRing from './ContextRing.vue';
import BalanceBadge from './BalanceBadge.vue';
import { accountFor, balanceFor, balances, refreshBalance, refreshBalanceIfStale } from './balance-store';
import { measuredUsage, type KnownWindow } from './context-meter';
import { effortFill as effortFillFor, effortLabel, effortStops as effortStopsFor, effortVivid as effortVividAt, modelEfforts, restingStop } from './reasoning-effort';
import { attachmentError, canSendMessage, composeMessage, formatBytes, MAX_ATTACHMENTS_PER_MESSAGE, pastedFile, type Attachment } from './attachment-state';
import { ACCESS_LABEL, approvalBody, approvalChoices, DECISION_LABEL, type ApprovalDecision } from './chat-approvals';
import { PERMISSION_LABELS, PERMISSION_NOTES, permissionName as permissionNameFor } from './permission-modes';
import { applyCommand, matchCommands, moveHighlight, slashQuery, unsupportedCommand } from './slash-commands';
import { MarkdownContent } from './MarkdownContent';
import { handoffArrivals, handoffChoices as handoffChoicesFor, handoffDraft, handoffTitle, profileNote, type HandoffChoice } from './handoff';
import { messageQueue, queuedContent, type QueuedMessage } from './message-queue';
import { lastChatSeq } from './chat-model';
import { attachedImagePaths } from './markdown-model';
import { conversationCache } from './conversation-cache';
import { createFrameBatch } from './frame-batch';
import { createStableRows } from './stable-items';
import { useConversationSearch } from './conversation-search';
import { useOlderHistory } from './older-history';
import { useSubagentDock } from './subagent-dock';
import type { Virtualizer } from '@tanstack/vue-virtual';
import VirtualRows from './VirtualRows.vue';
import { estimateRowHeight, rowHeights } from './row-heights';
import { followsAfterScroll, PERSON_WINDOW_MS, UPWARD_KEYS } from './scroll-follow';
import { filterModels, mergeModels, resolveModel, offeredModels } from './model-choices';
import { openLightbox } from './image-lightbox';
import { compactCount, descendants, inDock, lastLine, toolRuns, toolRunNames, toolIndicator, toolRunStatus, type ToolRun } from './tool-runs';
import { statusLabel as chatStatusLabel, statsLine as formatStats, turnUsageLine as formatTurnUsage, turnUsageTitle as formatTurnUsageTitle } from './chat-format';
import { mentionQuery, applyMention } from './file-mentions';
import { createFileSearch } from './file-search';
import type { SearchHit } from './file-search';
import { useI18n } from '../i18n';
import { CHORDS, formatChord, matches } from './shortcuts';
import { suggestionKey } from './chat-keyboard';
import { deliveryUncertain, pendingMessage, settleAccepted, settleFailed } from './message-delivery';
import { useSessionMenuPosition } from './session-menu-position';
import { canReopenInTerminal, terminalReopen } from './terminal-reopen';
import { randomId } from './random-id';
import CopyButton from './CopyButton.vue';
import { copyText, preserveNativeContextMenu } from './clipboard';

const props = defineProps<{ paneId?: string; session: Session; profiles: EndpointProfile[]; previewSnapshot?: ConversationSnapshot; consumeOpenIntent?: boolean }>();
const emit = defineEmits<{ changed: []; environment: [id: string]; renameRequest: [id: string]; legacy: []; profiles: []; openSession: [session: Session]; openFile: [reference: { path: string; line?: number; checkout?: string | null }] }>();
const { t, locale } = useI18n();
const isPreview = computed(() => props.previewSnapshot !== undefined);
const capabilities = backendCapabilities as typeof backendCapabilities & { structuredChat?: boolean; sessionConfiguration?: boolean; sessionTerminalEscape?: boolean };
const supported = computed(() => isPreview.value || capabilities.structuredChat === true);
const mode = ref<'structured' | 'pty' | undefined>(props.previewSnapshot?.mode), running = ref(props.previewSnapshot?.running ?? false), truncated = ref(!!props.previewSnapshot?.truncated), events = ref<ChatEvent[]>(props.previewSnapshot?.events ?? []);
const legacyAvailable = computed(() => !isPreview.value && (mode.value === 'pty' || mode.value === undefined && (props.session as Session & { interaction_mode?: string }).interaction_mode !== 'structured'));
// An escape hatch is only meaningful while structured mode is actually the
// surface in use, and only once the client has a conversation to reopen.
const terminalReopenAvailable = computed(() => !isPreview.value && mode.value === 'structured' && canReopenInTerminal(props.session, capabilities));
// Shown before it can be used too, disabled, so it is found before it is needed.
const terminalReopenOffered = computed(() => !isPreview.value && mode.value === 'structured' && capabilities.sessionTerminalEscape === true && props.session.provider !== 'terminal');
const mounted = ref(false), loading = ref(false), actionBusy = ref(false), error = ref(''), notice = ref('');
/**
 * Runs one of the pane's session actions with the pane marked busy. A failure
 * is shown only while the pane still shows the session it was for, and only
 * that session clears the busy mark, so switching sessions mid-request neither
 * shows a stale error nor unlocks the new one early.
 */
async function runAction(work: (id: string) => Promise<void>) {
  const id = props.session.id;
  actionBusy.value = true; error.value = '';
  try { await work(id); }
  catch (cause) { if (props.session.id === id && mounted.value) error.value = errorMessage(cause); }
  finally { if (props.session.id === id) actionBusy.value = false; }
}
const streamState = ref<SessionStreamState>(isPreview.value ? 'connected' : 'disconnected'), streamNotice = ref('');
const viewport = ref<HTMLElement>(), followBottom = ref(true), endpointConfirm = ref(false), selectedProfile = ref(props.session.endpoint_profile_id ?? '');
const confirmEnd = ref(false);
const sessionMenu = ref<HTMLDetailsElement>();
const composerInput = ref<HTMLTextAreaElement>();
const composing = ref(false);
const { tabStyle: tabMenuStyle, panelStyle: tabMenuPanelStyle, positionMenu } = useSessionMenuPosition(() => props.paneId, sessionMenu);
const approvalBusy = ref(''), approvalAnswers = reactive<Record<string, Record<string, string[]>>>(Object.create(null)), approvalOther = reactive<Record<string, Record<string, string>>>(Object.create(null));
const previewDrafts = createChatDraftStore();
const draft = computed(() => reactive((isPreview.value ? previewDrafts : chatDrafts).get(props.session.id)));
const view = computed(() => conversationView(events.value, running.value));
/** Tool calls made back to back read as one card; see tool-runs.ts. */
// A running subagent lives in the dock above the composer, not in the history;
// it takes its place there once it is done.
const displayItems = computed(() => toolRuns(running.value ? view.value.items.filter(item => !(item.type === 'tool' && inDock(item))) : view.value.items));
// Rows keep their identity while unchanged (stable-items.ts), so with v-memo a
// reply streaming into the last row re-renders that row, not the conversation.
const stabilize = createStableRows();
const rows = computed(() => stabilize(displayItems.value));
/**
 * The conversation is a window onto its rows, like OpenClaw's: only those near
 * the viewport are in the page, between two spacers sized from the heights the
 * others were measured at (or an estimate, until they are seen). Rows keep
 * their measured height by key, so older history put in front shifts nothing.
 */
const rowsMargin = ref(0), timelineWidth = ref(800);
// A row not yet seen is laid out at the height it was last measured at here
// (row-heights.ts), or an estimate from what it holds: the closer, the less
// the view is corrected as rows are measured while scrolling.
const rowHeight = (index: number) => {
  const key = rows.value.keys[index], item = rows.value.rows[index];
  return (key ? rowHeights.get(props.session.id, key, timelineWidth.value) : undefined) ?? (item ? estimateRowHeight(item, timelineWidth.value) : 160);
};
const rememberRowHeight = (index: number, height: number) => { const key = rows.value.keys[index]; if (key) rowHeights.remember(props.session.id, key, timelineWidth.value, height); };
/** The window onto the rows (VirtualRows.vue): it re-renders as the view scrolls, this pane does not. */
const rowsList = ref<{ virtualizer: Virtualizer<HTMLElement, Element>; $el: HTMLElement }>();
const windowed = () => rowsList.value?.virtualizer;
/** Where the rows begin inside the scrolled timeline, under what sits above them. */
function measureRowsMargin() {
  const el = viewport.value, rowsEl = rowsList.value?.$el;
  if (el && rowsEl) rowsMargin.value = Math.round(rowsEl.getBoundingClientRect().top - el.getBoundingClientRect().top + el.scrollTop);
}
// Rows measured while the view follows the newest output keep it at the bottom.
function onRowsResize() { if (followBottom.value) void scrollToLatest(); }

const { older, loadingOlder, olderSentinel, holds, loadOlder } = useOlderHistory({
  sessionId: () => props.session.id, preview: () => isPreview.value, mounted: () => mounted.value, generation: () => generation,
  events, viewport, windowed, rowKeys: () => rows.value.keys, measured: measureRowsMargin, failed: message => { notice.value = message; },
});
const { searchOpen, searchQuery, searchHits, searchNotice, searchInput, searchCount, openSearch, closeSearch, stepSearch, onRowsRange, searchKeydown } = useConversationSearch({
  sessionId: () => props.session.id, viewport, rows: () => rows.value.rows, windowed, holds, loadOlder, stopFollowing: () => { followBottom.value = false; }, t,
});
// A custom balance or an official allowance moves as turns are spent: read it
// again when one ends, and when this session first shows such an endpoint
// (both throttled).
watch(() => !!balanceFor(props.session.endpoint_profile_id) || !!accountFor(props.session.endpoint_profile_id), present => { if (present && !isPreview.value) refreshBalanceIfStale(props.session.endpoint_profile_id); }, { immediate: true });
watch(() => view.value.turn, (turn, before) => { if (before === 'running' && turn !== 'running' && !isPreview.value) refreshBalanceIfStale(props.session.endpoint_profile_id); });
// What sits above the rows -- the trimmed notice, the older-history marker,
// the welcome -- moves where they begin.
watch([truncated, () => older.value !== undefined, () => rows.value.rows.length === 0], () => { void nextTick(measureRowsMargin); });
onMounted(() => { void nextTick(measureRowsMargin); });
let widthObserver: ResizeObserver | undefined;
/** The "latest message" button floats just above where the timeline ends. A
 * fixed offset from the pane's foot lands on the composer whenever something
 * grows it -- a confirmation, a queue, a docked approval. */
const latestBottom = ref<number>();
watch(viewport, element => {
  widthObserver?.disconnect(); widthObserver = undefined;
  if (!element || typeof ResizeObserver !== 'function') return;
  widthObserver = new ResizeObserver(() => {
    const width = element.clientWidth; if (width && Math.abs(width - timelineWidth.value) >= 8) timelineWidth.value = width;
    const pane = element.closest('.chat-pane');
    if (pane) latestBottom.value = Math.max(12, pane.getBoundingClientRect().bottom - element.getBoundingClientRect().bottom + 12);
  });
  widthObserver.observe(element);
}, { immediate: true });
onBeforeUnmount(() => widthObserver?.disconnect());
/** Images load from this session's workspace, through the server, and nowhere else. */
const workspaceImage = (path: string) => assetUrl(props.session.workspace_id, path);
const hostImage = (path: string) => `/api/host/raw?${new URLSearchParams({ path })}`;
/** A file a message points at opens in the file pane; the shell resolves where. */
const openReference = (reference: { path: string; line?: number }) => emit('openFile', { ...reference, checkout: props.session.checkout_path });
/** While a run is going, its summary names the call in progress. */
function runningLine(run: ToolRun) {
  const current = [...run.tools].reverse().find(tool => tool.status === 'running');
  return current ? (current.activity ? lastLine(current.activity) : current.name) : toolRunNames(run);
}
const entry = computed(() => isPreview.value ? { epoch: 0 } : sessionConnections.get(props.session.id));
const matchingProfiles = computed(() => props.profiles.filter(profile => profile.provider === props.session.provider));
const activeProfile = computed(() => props.session.endpoint_snapshot ?? matchingProfiles.value.find(profile => profile.id === props.session.endpoint_profile_id));
const endpointName = computed(() => activeProfile.value?.name ?? t('Native · isolated configuration'));
/** What this client's headless mode does differently from its terminal, said before the first message. */
const chatNote = computed(() => agentClient(props.session.provider)?.chatNote);
const turnBusy = computed(() => view.value.turn === 'running' || view.value.awaitingApproval || !!draft.value.pending);
// Kept with the draft, like the text, so switching tabs does not drop them.
const attachments = computed<Attachment[]>({ get: () => draft.value.attachments ?? [], set: value => { draft.value.attachments = value; } });
const uploading = ref(false);
const caret = ref(0), commandIndex = ref(0), commandsDismissed = ref(false);
/** Suggestions exist only while a command is being typed and the client
 * advertised some. A client with no commands never opens a menu. */
const commandQuery = computed(() => commandsDismissed.value ? undefined : slashQuery(draft.value.text, caret.value));
const commandMatches = computed(() => commandQuery.value ? matchCommands(view.value.commands, commandQuery.value.term) : []);
const commandsOpen = computed(() => commandMatches.value.length > 0);
// A changed filter is a different list, so the highlight starts over instead of
// pointing at a stale row that may no longer exist.
watch(() => commandMatches.value.join('\u0000'), () => { commandIndex.value = 0; });
/**
 * `@path` completion, backed by the workspace index rather than this pane's
 * own knowledge — the composer has never loaded a file tree.
 *
 * Only one suggestion list is ever open: a slash command and a mention cannot
 * both be under the caret, and letting both claim the arrow keys would make
 * neither work.
 */
const mentionHits = ref<SearchHit[]>([]);
const mentionsDismissed = ref(false);
const mentionSearch = createFileSearch();
const mentionTarget = computed(() => (mentionsDismissed.value || commandsOpen.value) ? undefined : mentionQuery(draft.value.text, caret.value));
watch(() => mentionTarget.value?.term, term => {
  if (term === undefined) { mentionSearch.cancel(); mentionHits.value = []; return; }
  mentionSearch.search(props.session.workspace_id, term, results => { mentionHits.value = results?.files ?? []; });
});
const mentionsOpen = computed(() => !!mentionTarget.value && mentionHits.value.length > 0);
const mentionIndex = ref(0);
watch(() => mentionHits.value.map(hit => hit.path).join('\u0000'), () => { mentionIndex.value = 0; });
watch(() => draft.value.text, () => { mentionsDismissed.value = false; });
onBeforeUnmount(() => mentionSearch.cancel());
const { slowNow, agentStats, dock, dockOpen, expandedAgent, dockAgents, dockRunning, dockHeading, dismissAgent, clearFinished, logScrolled, quietLine } = useSubagentDock({ sessionId: () => props.session.id, running, items: () => view.value.items, t });
const statsLine = (stats: ToolStats) => formatStats(stats, t);
/** Below this a result is too small to matter to the context, and a size on
 * every card would only be noise. */
const NOTABLE_RESULT = 1000;
const sizeWorthShowing = (tool: ToolItem) => (tool.result_tokens ?? 0) >= NOTABLE_RESULT;
const turnUsageLine = (spent: TurnUsage) => formatTurnUsage(spent, t);
const turnUsageTitle = (spent: TurnUsage) => formatTurnUsageTitle(spent, t);
function chooseMention(hit: SearchHit | undefined) {
  const query = mentionTarget.value; if (!query || !hit) return;
  const next = applyMention(draft.value.text, query, hit.path);
  draft.value.text = next.text; mentionsDismissed.value = false; mentionIndex.value = 0; mentionHits.value = [];
  void nextTick(() => {
    const input = composerInput.value; if (!input) return;
    input.focus(); input.setSelectionRange(next.caret, next.caret); caret.value = next.caret;
  });
}

const fileInput = ref<HTMLInputElement>();
const OTHER_ANSWER = '__tendly_other__';
const canSendIgnoringText = computed(() => !isPreview.value && supported.value && mode.value === 'structured' && !loading.value && !actionBusy.value && !turnBusy.value && streamState.value === 'connected' && !uploading.value);
const canSend = computed(() => !isPreview.value && supported.value && mode.value === 'structured' && !loading.value && !actionBusy.value && !turnBusy.value && streamState.value === 'connected' && !uploading.value && canSendMessage(draft.value.text, attachments.value));
const selectedEffort = ref('');
/** A launch flag for this session only; it never rewrites a shared host configuration. */
const canTuneModel = computed(() => props.session.provider !== 'terminal');
/** The client's own model list for this session, empty until it announces one. */
/**
 * The model catalog, from the live client when it has spoken and from its
 * endpoint profile before that.
 *
 * A client only publishes its models once it is running, so a session that has
 * not started had nothing to offer. Claude papered over this with a hardcoded
 * list of levels; Codex publishes them per model, so the same guess would have
 * been an invention — and the control simply did not appear, which is the
 * asymmetry this removes. The profile endpoint asks the same client for the
 * same catalog without a session, so the answer is observed either way.
 */
const profileCatalog = ref<NativeModel[]>([]);
let catalogFetched = '';
watch([() => view.value.ready, () => activeProfile.value?.id, () => props.session.endpoint_profile_id], async () => {
  const id = props.session.endpoint_profile_id;
  // Asked even once the client is ready: the client lists only the models it
  // knows, and an endpoint behind a gateway serves many it does not.
  if (!id || catalogFetched === id) return;
  catalogFetched = id;
  try { profileCatalog.value = (await request<{ models?: NativeModel[] }>(`/endpoint-profiles/${id}/models`)).models ?? []; }
  // Staying silent is right: this only ever adds a control, and failing to add
  // one is the behaviour that already existed.
  catch { profileCatalog.value = []; }
}, { immediate: true });

// An official account's catalog is the client's own list; an endpoint's replaces it.
/**
 * The endpoint models the profile offers: the ones chosen in it, named as the
 * endpoint names them, or all it serves when none were chosen. Read from the
 * profile as it is now, so choosing more reaches sessions already open.
 */
/** The endpoint's models under the IDs this client runs them by. */
const endpointCatalog = computed(() => { const prefix = activeProfile.value?.native_config ? undefined : agentClient(props.session.provider)?.endpointModelPrefix; return prefix ? profileCatalog.value.map(entry => ({ ...entry, id: prefix + entry.id })) : profileCatalog.value; });
const offeredCatalog = computed(() => { const prefix = activeProfile.value?.native_config ? '' : agentClient(props.session.provider)?.endpointModelPrefix ?? ''; return offeredModels(endpointCatalog.value, (matchingProfiles.value.find(profile => profile.id === props.session.endpoint_profile_id) ?? activeProfile.value)?.models?.map(id => prefix + id)); });
const models = computed(() => mergeModels(view.value.models, activeProfile.value?.native_config ? [] : offeredCatalog.value, view.value.model ?? activeProfile.value?.model ?? undefined));
/** The window the server knows for the model in use (context-meter.ts weighs it against the client's). */
const modelContext = ref<KnownWindow>();
/** The menu's list, narrowed by what is typed below it: with an endpoint's hundred models, typing finds one. */
const visibleModels = computed(() => filterModels(models.value, customModel.value));
// Before the client speaks, the model in use is the profile's, or the one the
// catalog marks as its default — the same answer the client would give.
const currentModel = computed(() => view.value.model ?? activeProfile.value?.model
  ?? (view.value.ready ? undefined : models.value.find(entry => entry.isDefault)?.id) ?? undefined);
watch(currentModel, async model => {
  modelContext.value = undefined;
  if (!model) return;
  try {
    const info = await request<KnownWindow>(`/models/context?model=${encodeURIComponent(model)}`);
    if (currentModel.value === model) modelContext.value = info;
  } catch { /* An older backend: the client's own figure stands. */ }
}, { immediate: true });
const ringUsage = computed(() => measuredUsage(view.value.usage, modelContext.value));
const currentModelEntry = computed(() => models.value.find(entry => entry.id === currentModel.value));
/**
 * A model typed by hand rather than picked from the list.
 *
 * The list is whatever the host's client version knows about, so a model newer
 * than that client never appears in it even when the account can run it. The
 * client still accepts any ID and refuses one it cannot serve, so typing the ID
 * is the way onto it without waiting for a client upgrade.
 */
const customModel = ref('');
const customPicked = ref('');
const modelDefault = computed(() => currentModelEntry.value ? currentModelEntry.value.isDefault === true || currentModelEntry.value.id === 'default' : !customPicked.value || currentModel.value !== customPicked.value);
/**
 * The model chip always names the model in use, the default one too: which
 * model is answering is the first thing a reader of the conversation wants to
 * know. Whether it was chosen shows in the chip's emphasis, not its words.
 */
const shownModel = computed(() => currentModelEntry.value ?? (currentModel.value ? undefined : models.value.find(entry => entry.isDefault)));
const modelName = computed(() => (shownModel.value && resolvedDefaultName(shownModel.value, models.value)) ?? shownModel.value?.name ?? currentModel.value ?? t('Model'));
function pickCustomModel() {
  // A model's name typed by hand goes out as its ID, which is what the endpoint knows.
  const id = resolveModel(customModel.value, endpointCatalog.value);
  if (!id) return;
  customPicked.value = id; customModel.value = '';
  void pickModel(id);
}
const canPickModel = computed(() => canTuneModel.value && models.value.length > 0 && !isPreview.value && !actionBusy.value && !turnBusy.value && view.value.ready && running.value);
const manualEffort = computed(() => !!activeProfile.value && !activeProfile.value.native_config);
const effortChoices = computed(() => {
  const provider = props.session.provider;
  if (!canTuneModel.value || provider === 'terminal') return [];
  // Once the client has published its models, the levels come from the model
  // actually in use: one that supports none gets no depth control at all.
  const entry = currentModelEntry.value;
  if (entry && (entry.efforts?.length || !manualEffort.value)) return entry.efforts ?? [];
  return modelEfforts(provider, activeProfile.value?.model ?? undefined, undefined, selectedEffort.value || activeProfile.value?.effort || undefined, manualEffort.value);
});
const effortStops = computed(() => effortStopsFor(effortChoices.value));
const liveEffort = computed(() => view.value.effort ?? selectedEffort.value);
/**
 * A drag is one gesture, and the request belongs at the end of it.
 *
 * Every step used to send its own change, and the chip disables itself while
 * one is in flight -- so the slider went disabled under the finger and the drag
 * ended at the second stop, every time. The thumb follows the pointer locally
 * now, and the level is sent when it is let go.
 */
const heldEffort = ref<number>();
const effortIndex = computed(() => heldEffort.value ?? restingStop(effortStops.value, liveEffort.value));
/** What the slider is pointing at: where it is being held, or where it rests. */
const effortStop = computed(() => heldEffort.value === undefined ? liveEffort.value : effortStops.value[heldEffort.value] ?? '');
const effortName = computed(() => effortStop.value ? effortLabel(effortStop.value) : t('Thinking depth'));
/** Inside the panel there is room to say what the chip cannot: that nothing has
 * been chosen and the client is deciding. */
const effortSetting = computed(() => effortStop.value ? effortLabel(effortStop.value) : t('Client default'));
const effortFill = computed(() => effortFillFor(effortStops.value, effortIndex.value));
function dragEffort(event: Event) { heldEffort.value = Number((event.target as HTMLInputElement).value); }
function commitEffort(event: Event) {
  const stop = effortStops.value[Number((event.target as HTMLInputElement).value)];
  heldEffort.value = undefined;
  if (stop === undefined) return;
  selectedEffort.value = stop;
  // Stop 0 means "leave it to the client"; there is nothing to send for that.
  if (stop) void applyEffort(stop);
}
// Letting go without changing anything raises no `change` event, so the client
// reporting its level is the other thing that ends a hold.
watch(liveEffort, () => { heldEffort.value = undefined; });
const effortVivid = computed(() => effortVividAt(effortStops.value, effortIndex.value, effortStop.value));
const endpointMenu = ref<InstanceType<typeof ChipMenu>>();
const modelMenu = ref<InstanceType<typeof ChipMenu>>();
/** How tools get approved (permission-modes.ts). A change applies immediately:
 * both clients switch in place, so there is no context boundary to confirm and
 * no bridge restart to warn about. */
const permissionChoices = computed(() => view.value.permissionModes ?? []);
const livePermission = computed(() => view.value.permissionMode ?? '');
const permissionName = computed(() => permissionNameFor(livePermission.value, t));
const permissionDanger = computed(() => livePermission.value === 'danger');
/**
 * On a phone the composer keeps one row: attach, a single settings button and
 * send. The chips move into a sheet as full-width rows while it is open (a
 * Teleport, so each keeps its state), rather than scrolling sideways out of
 * sight under a thumb.
 */
const mobile = useMobile();
/** A touch-first device: its keyboard has no Shift+Enter, and Enter is how a
 * phone writes a paragraph. Judged by the pointer, not the width, so a tablet
 * counts and a narrow desktop window does not. */
const touchFirst = typeof window !== 'undefined' && !!window.matchMedia?.('(pointer: coarse)').matches;
const optionsOpen = ref(false), chipsInSheet = ref(false);
const optionsTarget = computed(() => 'chat-options-' + props.session.id.replace(/[^\w-]/g, '_'));
const optionsSummary = computed(() => [shownModel.value || currentModel.value ? modelName.value : '', effortStop.value ? effortName.value : ''].filter(Boolean).join(' · ') || endpointName.value);
function optionsClosed() { chipsInSheet.value = false; optionsOpen.value = false; }
const permissionMenu = ref<InstanceType<typeof ChipMenu>>();
async function pickPermission(mode: string) {
  permissionMenu.value?.close(true);
  if (!canConfigure.value || mode === livePermission.value) return;
  await runAction(async session => {
    const ack = await request<{ pending?: boolean }>(`${sessionPath(session)}/conversation/permission`, json('POST', { mode }));
    // Not running (stopped, or exited after a failed turn): the server keeps
    // the choice for the next start rather than refusing it.
    if (ack?.pending && props.session.id === session && mounted.value) notice.value = 'This permission mode applies when the session next starts.';
  });
}
async function pickModel(id: string) {
  modelMenu.value?.close(true);
  if (!canPickModel.value || id === currentModel.value) return;
  await runAction(async session => {
    // Only send a depth when one was actually chosen; otherwise the client
    // keeps whatever it is already using.
    await request(`${sessionPath(session)}/conversation/model`, json('POST', selectedEffort.value ? { model: id, effort: selectedEffort.value } : { model: id }));
    // The client answers on the event stream, not in this response: a refusal
    // arrives moments later as an error. Some endpoints reject the availability
    // probe a live switch makes, and for those the only way onto that model is
    // to start with it — which is offered rather than done silently, because it
    // begins a new context.
    await new Promise(resolve => setTimeout(resolve, 2500));
    if (props.session.id === session && mounted.value && view.value.model !== id) launchFallback.value = id;
  });
}
/**
 * Timeline context menu.
 *
 * Clearing is the client's own /clear, so it only appears when this client
 * advertised that command; Tendly never invents one.
 */
const timelineMenu = ref<{ x: number; y: number; text: string } | null>(null);
const clearAvailable = computed(() => canClearContext({ preview: isPreview.value, running: running.value, ready: view.value.ready, busy: turnBusy.value || actionBusy.value, connected: streamState.value === 'connected', commands: view.value.commands }));
function openTimelineMenu(event: MouseEvent) {
  if (isPreview.value || preserveNativeContextMenu(event, mobile.value)) return;
  event.preventDefault();
  timelineMenu.value = { x: event.clientX, y: event.clientY, text: String(window.getSelection() ?? '') };
}
function closeTimelineMenu() { timelineMenu.value = null; }
async function copyFromTimeline(value: string) {
  closeTimelineMenu();
  if (!value) return;
  try { await copyText(value); }
  catch { error.value = t('Could not copy to the clipboard in this browser.'); }
}
/** Sends the client's own /clear rather than wiping the view locally: the
 * transcript and the client's context have to go together, and only the client
 * can drop its own. */
async function clearContext() {
  closeTimelineMenu();
  if (!clearAvailable.value) return;
  const messageId = randomId();
  await runAction(async id => { await request(`${sessionPath(id)}/conversation/message`, json('POST', { id: messageId, content: '/clear' })); });
}
/** A model the live switch could not reach, offered as a launch choice. */
const launchFallback = ref('');
const launchFallbackName = computed(() => models.value.find(entry => entry.id === launchFallback.value)?.name ?? launchFallback.value);
async function applyModelAtLaunch() {
  const model = launchFallback.value;
  if (!model || !canConfigure.value) return;
  await runAction(async id => {
    const payload = sessionConfigurationPayload(props.session, props.profiles, selectedProfile.value || null, true, { mode: mode.value, ready: view.value.ready, busy: view.value.turn === 'running', awaitingApproval: view.value.awaitingApproval }, { model, effort: selectedEffort.value });
    await request<Session>(`${sessionPath(id)}/configuration`, json('PATCH', payload));
    if (props.session.id === id && mounted.value) { launchFallback.value = ''; notice.value = 'Endpoint changed. The next message starts a new native context and takes the conversation so far with it.'; emit('changed'); await load(); }
  });
}
/**
 * Ways to begin, for a conversation that has none yet.
 *
 * Each only fills the composer: what is sent is still the user's to read and
 * change first. Focusing the box is also what wakes the client, so choosing
 * one starts it the same way clicking into the box would.
 */
const STARTERS = [
  { icon: 'folder', title: 'Map out this project', note: 'Structure, entry points and how the pieces fit', prompt: 'Give me a tour of this project: its structure, entry points and how the main pieces fit together.' },
  { icon: 'git', title: 'Review my changes', note: 'What changed, and what looks risky', prompt: 'Review the uncommitted changes in this workspace and point out anything risky.' },
  { icon: 'search', title: 'Hunt for bugs', note: 'Likely bugs and unhandled edge cases', prompt: 'Look through the code for likely bugs or unhandled edge cases, and list the most important ones first.' },
  { icon: 'check', title: 'Add tests', note: 'Find weak coverage and propose tests', prompt: 'Find an important piece of code with weak test coverage and propose tests for it.' },
] as const;
// Hidden once there is a draft: choosing one replaces the composer's text, and
// that text may be a handed-over conversation.
const startersAvailable = computed(() => supported.value && structuredSession.value && !draft.value.pending && !draft.value.text.trim());
function useStarter(prompt: string) {
  if (!startersAvailable.value) return;
  draft.value.text = t(prompt);
  void nextTick(() => {
    const input = composerInput.value; if (!input) return;
    input.focus(); input.setSelectionRange(input.value.length, input.value.length); caret.value = input.value.length;
  });
}
/**
 * Continuing this conversation with another agent.
 *
 * The clients cannot read each other's histories, so this opens a new
 * session of another client with the conversation written into its message
 * box (see handoff.ts). Nothing is sent until the user has read it and said
 * what to do next, and this session is left exactly as it is.
 */
const handoffChoices = computed(() => handoffChoicesFor(props.session.provider, props.profiles, t));
const handoffTarget = ref<HandoffChoice>();
function pickHandoff(choice: HandoffChoice) {
  endpointMenu.value?.close(true);
  if (!isPreview.value) handoffTarget.value = choice;
}
async function confirmHandoff() {
  const choice = handoffTarget.value, source = props.session;
  if (!choice || actionBusy.value) return;
  await runAction(async () => {
    const messages = view.value.items.flatMap(item => item.type === 'message' ? [{ role: item.role, text: item.text }] : []);
    const created = await request<Session>(`${workspacePath(choice.workspaceId ?? source.workspace_id)}/sessions`, json('POST', { title: handoffTitle(source.title, choice, t), provider: choice.provider, endpoint_profile_id: choice.profileId, interaction_mode: 'structured' }));
    const text = handoffDraft(messages, source.provider, choice, t);
    if (text) chatDrafts.get(created.id).text = text;
    handoffTarget.value = undefined; handoffArrivals.add(created.id);
    emit('changed'); emit('openSession', created);
  });
}
/**
 * The branch this session works on, and moving it to another.
 *
 * Each session has a checkout: the workspace directory, or a git worktree of
 * the same repository beside it. Choosing a branch moves this session -- and
 * only this one -- into that branch's worktree, made if it does not exist.
 * The client restarts there in a new native context, as with an endpoint
 * change; earlier messages stay on screen. Switching every session's branch
 * at once is the Git pane's job.
 */
const repoState = ref<{ current?: string | null; branches: string[] }>();
const branchTarget = ref(''), branchMenu = ref<InstanceType<typeof ChipMenu>>(), branchConfirm = ref('');
async function loadRepo() {
  try { repoState.value = await request(`${workspacePath(props.session.workspace_id)}/git/branches`); }
  catch { repoState.value = undefined; }
}
// After mount, never while rendering: a pane that is only drawn -- a preview,
// a server render -- asks nothing of the host.
onMounted(() => { if (!isPreview.value) void loadRepo(); });
watch(() => props.session.workspace_id, () => { repoState.value = undefined; if (mounted.value && !isPreview.value) void loadRepo(); });
const sessionBranch = computed(() => props.session.checkout_path ? props.session.checkout_branch ?? undefined : repoState.value?.current ?? undefined);
const branchChoices = computed(() => repoState.value?.branches ?? []);
function pickBranch(name: string) {
  name = name.trim(); branchMenu.value?.close(true);
  if (!name || name === sessionBranch.value || actionBusy.value) return;
  // Moving starts a new native context; with nothing said yet there is nothing to lose.
  if (view.value.items.some(item => item.type === 'message')) branchConfirm.value = name;
  else void moveToBranch(name);
}
async function moveToBranch(branch: string) {
  branchConfirm.value = '';
  await runAction(async id => {
    await request<Session>(`${sessionPath(id)}/checkout`, json('POST', { branch, create: !branchChoices.value.includes(branch) }));
    branchTarget.value = '';
    if (props.session.id === id && mounted.value) { emit('changed'); void loadRepo(); }
  });
}
function pickEndpoint(id: string) {
  if (!canConfigure.value) return;
  selectedProfile.value = id; chooseEndpoint(); endpointMenu.value?.close(true);
}
/** The chip opens even while this session is busy: changing its endpoint waits,
 * but moving to another client opens a new session and never has to. */
const canOpenEndpoint = computed(() => !isPreview.value && (canConfigure.value || handoffChoices.value.length > 0));
const canConfigure = computed(() => !isPreview.value && capabilities.sessionConfiguration === true && !loading.value && !actionBusy.value && !turnBusy.value && (['stopped', 'failed'].includes(props.session.status) || mode.value === 'structured' && running.value && view.value.ready));
/**
 * The client is on its way up: a start was asked for, or the process is running
 * but has not finished its handshake. Its model list, permissions and commands
 * all arrive with that handshake, so until then the composer says it is
 * starting instead of looking half-built.
 */
const structuredSession = computed(() => mode.value === 'structured' || mode.value === undefined && props.session.interaction_mode === 'structured');
const booting = computed(() => !isPreview.value && supported.value && structuredSession.value && !view.value.ready && !error.value
  && (!!(entry.value as { pending?: unknown }).pending || running.value || loading.value && ['starting', 'running', 'waiting'].includes(props.session.status)));
const statusLabel = computed(() => chatStatusLabel({
  preview: isPreview.value,
  booting: booting.value,
  loading: loading.value,
  awaitingApproval: view.value.awaitingApproval,
  working: view.value.turn === 'running',
  disconnected: streamState.value === 'error' || streamState.value === 'disconnected',
  structured: mode.value === 'structured',
}));
let generation = 0, readController: AbortController | undefined, stream: ReturnType<typeof createSessionStream> | undefined, stopBatch = () => {};

// Whether the view keeps following the newest output (scroll-follow.ts): a
// person scrolling up stops it at once, coming back near the bottom resumes it.
let lastScrollTop = 0, personAt = 0;
function byPerson() { personAt = performance.now(); }
function atBottom() {
  const el = viewport.value;
  if (!el) { followBottom.value = true; return; }
  const movedUp = el.scrollTop < lastScrollTop - 1; lastScrollTop = el.scrollTop;
  followBottom.value = followsAfterScroll({ following: followBottom.value, movedUp, byPerson: performance.now() - personAt < PERSON_WINDOW_MS, gap: el.scrollHeight - el.scrollTop - el.clientHeight });
}
// Each is decided before the scroll it causes lands, so a reply streaming in
// cannot pull the view back down in between.
function timelineWheel(event: WheelEvent) { byPerson(); if (event.deltaY < 0) followBottom.value = false; }
let touchY: number | undefined;
function timelineTouchStart(event: TouchEvent) { byPerson(); touchY = event.touches[0]?.clientY; }
function timelineTouchMove(event: TouchEvent) { byPerson(); const y = event.touches[0]?.clientY; if (touchY !== undefined && y !== undefined && y > touchY + 4) followBottom.value = false; }
function timelineKeydown(event: KeyboardEvent) { if (event.metaKey || event.ctrlKey || event.altKey) return; byPerson(); if (UPWARD_KEYS.has(event.key)) followBottom.value = false; }
/** Whether the person is selecting text in the conversation: following new output would pull it away. */
function selectingInTimeline(el: HTMLElement) { const selection = window.getSelection(); return !!selection && !selection.isCollapsed && !!selection.anchorNode && el.contains(selection.anchorNode); }
// Asked once a frame while a reply streams: the selection is only looked at,
// never turned into text, which cost half the frame on a long conversation.
async function scrollToLatest(force = false) {
  await nextTick(); const el = viewport.value;
  if (!el || !(force || followBottom.value) || (!force && selectingInTimeline(el))) return;
  // The virtualizer goes to the last row and keeps correcting as the rows it
  // brings in are measured; what follows the rows (the working line) is below.
  const count = rows.value.rows.length;
  if (count) windowed()?.scrollToIndex(count - 1, { align: 'end' });
  el.scrollTop = el.scrollHeight; followBottom.value = true;
}
function updateSnapshot(raw: unknown) {
  const snapshot = parseConversationSnapshot(raw);
  // A resumed greeting carries only what follows the view's last event.
  const bounded = pruneChatEvents(snapshot.after === undefined ? snapshot.events : snapshot.events.reduce(appendChatEvent, events.value));
  mode.value = snapshot.mode; running.value = snapshot.running; truncated.value = !!snapshot.truncated || bounded.truncated; events.value = bounded.events;
  // A latest page says where older history begins; a resumed greeting keeps the cursor the view had.
  if (snapshot.after === undefined) older.value = snapshot.older;
  if (bounded.dropped !== undefined) older.value = Math.max(older.value ?? 0, bounded.dropped + 1);
  acknowledgeDraft(draft.value, snapshot.events);
  if (!snapshot.running) clearAnswers();
  for (const event of snapshot.events) if (event.type === 'approval_resolved') clearAnswers(event.id);
  void scrollToLatest();
}
function attach(id: string, own: number) {
  stream?.dispose();
  const current = () => mounted.value && own === generation && props.session.id === id;
  // A reply streams dozens of pieces a second: they are applied once a frame,
  // in order, so the view is rebuilt once a frame rather than once a piece.
  const batch = createFrameBatch<ChatEvent>(messages => {
    if (!current()) return;
    let next = events.value;
    for (const message of messages) next = appendChatEvent(next, message);
    const bounded = pruneChatEvents(next);
    events.value = bounded.events; truncated.value ||= bounded.truncated;
    if (bounded.dropped !== undefined) older.value = Math.max(older.value ?? 0, bounded.dropped + 1);
    acknowledgeDraft(draft.value, events.value);
    for (const message of messages) {
      if (message.type === 'ready') { running.value = true; emit('changed'); }
      if (message.type === 'exit') { running.value = false; emit('changed'); }
      if (message.type === 'approval_resolved') clearAnswers(message.id);
      if (message.type === 'configuration' || message.type === 'exit') clearAnswers();
      // A turn starting is also worth reporting: the session takes its name from
      // the message that opened it, and the sidebar is where that name is read.
      if (message.type === 'turn') emit('changed');
    }
    void scrollToLatest();
  });
  stopBatch = () => batch.clear();
  stream = createSessionStream({
    createSocket: url => new WebSocket(url),
    // A dropped view re-attaches on its own; the server greets every socket
    // with everything after the last event the view holds, so nothing missed
    // meanwhile is lost and nothing it already has is sent again.
    reconnect: true,
    hidden: () => document.hidden,
    onState: (state, detail) => { if (current()) { streamState.value = state; streamNotice.value = detail; } },
    onMessage: raw => {
      if (!current() || typeof raw !== 'string') return;
      if (new TextEncoder().encode(raw).byteLength > 16 * 1024 * 1024) throw Error('Invalid conversation response.');
      const message: unknown = JSON.parse(raw);
      // A snapshot replaces or extends what the queue builds on: what is
      // waiting goes first, so nothing is applied out of order.
      if (message && typeof message === 'object' && (message as { type?: string }).type === 'snapshot') { batch.flush(); updateSnapshot(message); return; }
      if (!isChatEvent(message)) throw Error('Invalid conversation response.');
      batch.push(message);
    },
  });
  const url = new URL(`/api${sessionPath(id)}/chat/ws`, window.location.href); url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  stream.connect(() => { const after = lastChatSeq(events.value); if (after === undefined) url.searchParams.delete('after'); else url.searchParams.set('after', String(after)); return url.href; });
}
async function load(consumeOpenIntent = false) {
  const id = props.session.id, own = ++generation;
  readController?.abort(); stream?.dispose(); stopBatch(); readController = new AbortController();
  if (!mounted.value || !supported.value) { loading.value = false; return; }
  if (props.previewSnapshot) { updateSnapshot(props.previewSnapshot); loading.value = false; streamState.value = 'connected'; streamNotice.value = ''; error.value = ''; return; }
  // A conversation shown before is shown again at once, and its socket
  // resumes after the last event it holds: what happened meanwhile, and the
  // session's state now, arrive in that greeting. No empty pane in between.
  const cached = conversationCache.get(id);
  if (cached) { events.value = cached.events; mode.value = cached.mode; running.value = cached.running; truncated.value = cached.truncated; older.value = cached.older; void scrollToLatest(true); }
  loading.value = !cached; error.value = ''; streamNotice.value = ''; streamState.value = 'connecting';
  try {
    // ensure only consumes the parent shell's explicit open intent. A normal
    // stopped-session remount has no permission to start a process.
    if (consumeOpenIntent) { await sessionConnections.ensure(props.session); if (own !== generation) return; emit('changed'); }
    if (cached) { attach(id, own); return; }
    const snapshot = await request<unknown>(`${sessionPath(id)}/conversation`, { signal: readController.signal });
    if (!mounted.value || own !== generation || props.session.id !== id) return;
    updateSnapshot(snapshot);
    // A legacy PTY session is rendered in this shell but its terminal stream
    // belongs to NativeSessionPane. Do not attach the structured socket or
    // turn a view mount into a start request; the user must explicitly enable
    // conversation mode first.
    if (mode.value === 'structured') attach(id, own); else streamState.value = 'disconnected';
  } catch (cause) { if (own === generation && mounted.value && !(cause instanceof Error && cause.name === 'AbortError')) { error.value = t(errorMessage(cause)); streamState.value = 'error'; } }
  finally { if (own === generation) loading.value = false; }
}
watch([() => props.session.id, () => props.session.interaction_mode, () => props.session.configuration_revision, () => entry.value.epoch, () => props.previewSnapshot, mounted, supported], () => {
  clearAnswers();
  wakeAttempted = ''; launchFallback.value = ''; handoffTarget.value = undefined;
  mode.value = undefined; events.value = []; older.value = undefined; endpointConfirm.value = false; confirmEnd.value = false; approvalBusy.value = ''; actionBusy.value = false; notice.value = ''; selectedProfile.value = props.session.endpoint_profile_id ?? ''; followBottom.value = true;
  void load(props.consumeOpenIntent !== false && props.session.interaction_mode === 'structured');
});
watch(() => props.session.endpoint_profile_id, id => { if (!endpointConfirm.value) selectedProfile.value = id ?? ''; });
// What this view shows is kept for when it is shown again (conversation-cache.ts).
watch([events, mode, running, truncated, older], () => { if (!isPreview.value) conversationCache.remember(props.session.id, { events: events.value, mode: mode.value, running: running.value, truncated: truncated.value, older: older.value }); });
onMounted(() => {
  mounted.value = true;
  if (!handoffArrivals.delete(props.session.id)) return;
  void nextTick(() => {
    const input = composerInput.value; if (!input) return;
    const end = input.value.length;
    input.focus(); input.setSelectionRange(end, end); input.scrollTop = input.scrollHeight; caret.value = end;
  });
});
/**
 * Coming back to the page re-reads the conversation and re-attaches a view
 * that dropped meanwhile, or that may only look attached after a long absence.
 * `load()` without an open intent never starts a process or resends anything.
 */
let stopPageReturn = () => {};
onMounted(() => {
  stopPageReturn = onPageReturn(stale => {
    if (!mounted.value || isPreview.value || loading.value || actionBusy.value || mode.value !== 'structured') return;
    // A view that gave up re-reads the conversation; a live or retrying one is
    // woken, which replaces a stale socket and probes one that looks open.
    if (streamState.value === 'disconnected' || streamState.value === 'error') void load(); else stream?.wake(stale);
  });
});
onBeforeUnmount(() => { stopPageReturn(); mounted.value = false; generation++; readController?.abort(); stream?.dispose(); stopBatch(); });

/** Focusing the composer is an intent to use this session, unlike restoring a
 * layout. Waking here is what makes the model list and the client's own
 * commands available before the first message rather than after it: both are
 * announced at startup, and starting sends no prompt, so nothing is spent. */
let wakeAttempted = '';
async function wake() {
  if (isPreview.value || !supported.value || running.value || loading.value) return;
  if (props.session.interaction_mode !== 'structured' && mode.value !== 'structured') return;
  const id = props.session.id;
  if (wakeAttempted === id || sessionConnections.get(id).pending) return;
  wakeAttempted = id;
  sessionConnections.requestOpen(id);
  await load(true);
}
/**
 * A message written while the agent works has three ways out, in order of
 * how often each is wanted:
 * - steer (Enter): into the running turn, read at its next step;
 * - queue (⌥/Alt+Enter): after the turn, as a turn of its own;
 * - interrupt and send: stop the turn and send it now.
 * Steering a turn that has just ended is simply sending; one the client
 * cannot take is queued rather than lost. See message-queue.ts for the queue.
 */
const queued = computed(() => messageQueue.get(props.session.id));
const canQueue = computed(() => !isPreview.value && supported.value && mode.value === 'structured' && view.value.turn === 'running' && !uploading.value && canSendMessage(draft.value.text, attachments.value));
/** The draft as a waiting message, taken out of the composer. */
function takeDraft(): QueuedMessage {
  const target = draft.value, item: QueuedMessage = { id: randomId(), text: target.text, ...(target.attachments?.length ? { attachments: target.attachments } : {}) };
  target.text = ''; target.attachments = [];
  return item;
}
function queueDraft() {
  if (!canQueue.value) return;
  messageQueue.push(props.session.id, takeDraft());
}
const steering = ref(false);
async function steerText(text: string): Promise<boolean> {
  const id = props.session.id, content = text.trim();
  // Already composed: any attachments are named in it.
  if (!content || steering.value) return false;
  steering.value = true; error.value = '';
  try { await request(`${sessionPath(id)}/conversation/steer`, json('POST', { id: randomId(), content })); return true; }
  catch (cause) { if (props.session.id === id) error.value = errorMessage(cause); return false; }
  finally { steering.value = false; }
}
async function steerDraft() {
  if (!canQueue.value) return;
  const item = takeDraft();
  // Refused -- the connection dropped, say -- it waits in the queue instead of vanishing.
  if (!(await steerText(queuedContent(item)))) messageQueue.push(props.session.id, item);
}
function interruptWithDraft() {
  if (!canQueue.value) return;
  messageQueue.get(props.session.id).unshift(takeDraft());
  steerMenu.value?.close(true);
  void interrupt();
}
async function steerQueued(id: string) {
  const item = messageQueue.remove(props.session.id, id);
  if (item && !(await steerText(queuedContent(item)))) messageQueue.get(props.session.id).unshift(item);
}
const steerMenu = ref<InstanceType<typeof ChipMenu>>();
function editQueued(id: string) {
  const item = messageQueue.remove(props.session.id, id);
  if (!item) return;
  draft.value.text = draft.value.text.trim() ? `${draft.value.text}\n${item.text}` : item.text;
  if (item.attachments?.length) draft.value.attachments = [...(draft.value.attachments ?? []), ...item.attachments];
  void nextTick(() => composerInput.value?.focus());
}
function sendQueuedNow(id: string) {
  const list = messageQueue.get(props.session.id), index = list.findIndex(item => item.id === id);
  if (index > 0) list.unshift(...list.splice(index, 1));
  void interrupt();
}
// The turn ending is what releases the next message. Checked against the same
// conditions a manual send is, so nothing goes out over an approval or while
// the previous message is still unconfirmed.
watch(() => [canSendIgnoringText.value, queued.value.length] as const, ([ready, count]) => {
  if (!ready || !count || draft.value.pending) return;
  const next = messageQueue.shift(props.session.id); if (!next) return;
  const id = props.session.id, target = draft.value, messageId = randomId();
  target.pending = pendingMessage(messageId, queuedContent(next), target.text, props.session); target.notice = undefined; error.value = '';
  void deliver(id, target, () => messageQueue.get(id).unshift(next));
});
async function send(queue = false) {
  if (canQueue.value) { if (queue) queueDraft(); else await steerDraft(); return; }
  if (!canSend.value) return;
  // This client exposes no commands, so a bare slash command would reach the
  // model as prose. Say so instead of sending something that cannot work.
  const unsupported = unsupportedCommand(draft.value.text, view.value.commands, view.value.commandsAnnounced);
  // Codex has no slash commands in its protocol at all; what its terminal
  // offers as /model and /approvals are the controls beside this box here.
  if (unsupported) { draft.value.notice = undefined; error.value = t('{command} is not a command this client accepts. Model and thinking depth are the controls below; anything else belongs in the native terminal view.', { command: '/' + unsupported }); return; }
  const id = props.session.id, target = draft.value, messageId = randomId();
  // Attachment paths travel with the text, so an accepted receipt covers both.
  const typed = target.text;
  const content = composeMessage(typed, target.attachments ?? []);
  target.attachments = [];
  target.pending = pendingMessage(messageId, content, typed, props.session); target.notice = undefined; error.value = '';
  await deliver(id, target);
}
async function pickAttachments(event: Event) {
  const input = event.target as HTMLInputElement;
  const files = [...(input.files ?? [])];
  input.value = '';
  await uploadFiles(files);
}
/**
 * Files dropped on, or pasted into, the composer.
 *
 * A pasted screenshot arrives as a file with no name of its own, so one is
 * built from its type rather than sending the browser's "image.png" for every
 * paste and overwriting the last one.
 */
async function pasteFiles(event: ClipboardEvent) {
  const files = [...(event.clipboardData?.files ?? [])];
  if (isPreview.value || !files.length) return;
  // Preserve native insertion (including the caret/selection and undo) for mixed
  // text + image clipboards; suppress only a file-only paste.
  if (!event.clipboardData?.getData('text/plain')) event.preventDefault();
  await uploadFiles(files.map(file => pastedFile(file, new Date())));
}
async function dropFiles(event: DragEvent) {
  const files = [...(event.dataTransfer?.files ?? [])];
  if (isPreview.value || !files.length) return;
  event.preventDefault();
  await uploadFiles(files);
}
async function uploadFiles(files: File[]) {
  if (isPreview.value || !files.length) return;
  uploading.value = true; error.value = '';
  // An upload that finishes after a tab switch still lands on its own session's draft.
  const target = draft.value;
  try {
    for (const file of files) {
      const reason = attachmentError(file, target.attachments?.length ?? 0);
      if (reason) { error.value = t(reason, { count: MAX_ATTACHMENTS_PER_MESSAGE }); break; }
      const saved = await request<Attachment>(
        `${workspacePath(props.session.workspace_id)}/attachments?name=${encodeURIComponent(file.name)}`,
        { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-Tendly-Client': 'web' }, body: file },
      );
      target.attachments = [...(target.attachments ?? []), saved];
    }
  } catch (cause) { error.value = t(errorMessage(cause)); }
  finally { uploading.value = false; }
}
/** Removing a chip only drops the reference; the uploaded file stays in the
 * workspace where the user can see and delete it. */
function removeAttachment(path: string) { attachments.value = attachments.value.filter(item => item.path !== path); }

async function retryDelivery() {
  if (isPreview.value || loading.value || actionBusy.value || streamState.value !== 'connected') return;
  const target = draft.value;
  try { pendingMessageRetry(target, props.session); target.pending!.state = 'sending'; target.notice = undefined; await deliver(props.session.id, target); }
  catch (cause) { error.value = t(errorMessage(cause)); }
}
/** `requeue` is given for a message released from the queue: refused outright,
 * it goes back where the user put it rather than into the composer (which it
 * was never in), the way the Steer actions restore theirs. */
async function deliver(id: string, target: ChatDraft, requeue?: () => void) {
  const pending = target.pending; if (!pending || isPreview.value) return;
  const { id: messageId, content } = pending;
  error.value = '';
  try {
    const receipt = await request(`${sessionPath(id)}/conversation/message`, json('POST', { id: messageId, content, configuration_revision: pending.configurationRevision ?? 0 }));
    settleAccepted(target, messageId, content, receipt);
    if (props.session.id === id && mounted.value) { emit('changed'); void load(); }
  } catch (cause) {
    // A refusal is final (the turn was still running, say); an uncertain
    // delivery may have landed, so that one stays pending to be resolved by
    // hand, never re-queued, lest it send twice.
    if (requeue && !deliveryUncertain(cause)) { if (target.pending?.id === messageId) { target.pending = undefined; target.notice = undefined; } requeue(); }
    else settleFailed(target, messageId, content, cause);
    if (props.session.id === id && mounted.value) error.value = errorMessage(cause);
  }
}
async function explicitOpen() {
  if (isPreview.value || actionBusy.value || loading.value || !supported.value || mode.value === 'pty' && running.value) return;
  await runAction(async id => { await request(`${sessionPath(id)}/conversation/open`, json('POST', {})); if (props.session.id === id && mounted.value) { emit('changed'); await load(); } });
}
/**
 * Open this conversation in a real terminal, for the interactive commands the
 * structured pipe cannot carry. The new session is a sibling, not a
 * replacement: this pane keeps its history and stays exactly where it is.
 */
async function openInTerminal() {
  if (isPreview.value || actionBusy.value || !terminalReopenAvailable.value) return;
  await runAction(async id => {
    const opened = await terminalReopen(props.session);
    if (props.session.id === id && mounted.value) { emit('changed'); emit('openSession', opened); }
  });
}
async function interrupt() {
  if (isPreview.value || actionBusy.value || view.value.turn !== 'running') return;
  await runAction(async id => { await request(`${sessionPath(id)}/conversation/interrupt`, json('POST', {})); if (props.session.id === id && mounted.value) { emit('changed'); await load(); } });
}
async function endSession() {
  if (isPreview.value || actionBusy.value || !confirmEnd.value) return;
  await runAction(async id => {
    await request(`${sessionPath(id)}/stop`, json('POST'));
    sessionConnections.ended(id);
    if (props.session.id === id && mounted.value) { confirmEnd.value = false; emit('changed'); await load(); }
  });
}
function closeSessionMenu(restoreFocus = false) {
  const menu = sessionMenu.value;
  if (!menu) return;
  menu.open = false;
  if (restoreFocus) menu.querySelector<HTMLElement>('summary')?.focus({ preventScroll: true });
}
function requestEnd() { closeSessionMenu(); confirmEnd.value = true; }
function cancelEndpointChange() { endpointConfirm.value = false; selectedProfile.value = props.session.endpoint_profile_id ?? ''; }
/**
 * A key typed in the conversation but outside any field -- after Esc left the
 * composer, or a click in the timeline -- goes to the composer rather than
 * nowhere, and "/" there opens the commands as it would in the composer.
 */
function typeIntoComposer(event: KeyboardEvent): boolean {
  const input = composerInput.value, target = event.target instanceof Element ? event.target : null;
  if (!input || input.disabled || isPreview.value || event.defaultPrevented || event.ctrlKey || event.metaKey || event.altKey) return false;
  if (!target || target === input || target.closest('input, textarea, select, [contenteditable="true"], details, [role="menu"], [role="dialog"]')) return false;
  if (event.isComposing || event.key === 'Process') { input.focus({ preventScroll: true }); return true; }
  if (event.key === 'Enter' && !target.closest('button, a, summary, [role="button"], [role="option"]')) {
    event.preventDefault(); input.focus({ preventScroll: true }); return true;
  }
  if (event.key.length !== 1 || event.key === ' ') return false;
  event.preventDefault();
  draft.value.text += event.key;
  void nextTick(() => {
    const end = input.value.length;
    input.focus({ preventScroll: true });
    input.setSelectionRange(end, end);
    caret.value = end;
  });
  return true;
}
/**
 * The request in the dock answers to the conversation's verbs: confirm accepts
 * it, or submits the answers, and remove declines it -- from the composer too,
 * where remove still edits text while there is any.
 */
function answerFromKeys(event: KeyboardEvent): boolean {
  const item = dockedApproval.value;
  if (!item || isPreview.value || approvalBusy.value || composing.value || streamState.value !== 'connected') return false;
  const choices = approvalChoices(item);
  const answer = matches(event, CHORDS.confirm) && choices.includes('accept') && !incompleteAnswers(item) ? 'accept'
    : matches(event, CHORDS.remove) && choices.includes('decline') && !(event.target === composerInput.value && draft.value.text) ? 'decline' : undefined;
  if (!answer) return false;
  event.preventDefault(); event.stopPropagation();
  void approve(item, answer);
  return true;
}
function paneKeydown(event: KeyboardEvent) {
  if (typeIntoComposer(event)) return;
  // Find searches the whole conversation: the browser's own find sees only the rows in the page.
  if (matches(event, CHORDS.find) && !isPreview.value) {
    event.preventDefault(); event.stopPropagation(); openSearch(); return;
  }
  if (answerFromKeys(event)) return;
  if (event.key !== 'Escape' || event.defaultPrevented || composing.value) return;
  if (sessionMenu.value?.open) { event.preventDefault(); event.stopPropagation(); closeSessionMenu(true); return; }
  if (confirmEnd.value) { event.preventDefault(); event.stopPropagation(); confirmEnd.value = false; composerInput.value?.focus({ preventScroll: true }); return; }
  if (endpointConfirm.value) { event.preventDefault(); event.stopPropagation(); cancelEndpointChange(); composerInput.value?.focus({ preventScroll: true }); return; }
  // Interrupting is the composer's Esc, as in the client's own terminal; from the timeline it would be too easy to do by accident.
  if (event.target === composerInput.value && !isPreview.value && view.value.turn === 'running' && !view.value.awaitingApproval) { event.preventDefault(); event.stopPropagation(); void interrupt(); return; }
  // Leaving the composer stays in the conversation, where the next key typed
  // comes back to it, rather than dropping focus on the page.
  if (event.target === composerInput.value) { event.preventDefault(); event.stopPropagation(); viewport.value?.focus({ preventScroll: true }); }
}
/**
 * Requests waiting on you sit above the composer, where your attention already
 * is, one at a time; the timeline keeps a one-line record of each.
 */
const pendingApprovals = computed(() => view.value.items.filter((item): item is Extract<ChatItem, { type: 'approval' }> => item.type === 'approval' && !item.resolved));
const approvalIndex = ref(0);
const dockedIndex = computed(() => Math.max(0, Math.min(approvalIndex.value, pendingApprovals.value.length - 1)));
const dockedApproval = computed(() => pendingApprovals.value[dockedIndex.value]);
/** Requests folded to their title line, to read the conversation above them. A new request arrives open. */
const foldedApprovals = reactive(new Set<string>());
function toggleApprovalFold(id: string) { if (!foldedApprovals.delete(id)) foldedApprovals.add(id); }
const acceptKey = formatChord(CHORDS.confirm), declineKey = formatChord(CHORDS.remove), findKey = formatChord(CHORDS.find);
function clearAnswers(id?: string) { for (const key of id ? [id] : new Set([...Object.keys(approvalAnswers), ...Object.keys(approvalOther)])) { delete approvalAnswers[key]; delete approvalOther[key]; } }
function answer(id: string, question: string, event: Event) { const value = (event.target as HTMLInputElement).value; (approvalAnswers[id] ??= Object.create(null))[question] = value && value !== OTHER_ANSWER ? [value] : []; if (value !== OTHER_ANSWER && approvalOther[id]) delete approvalOther[id][question]; }
function toggleAnswer(id: string, question: string, value: string, event: Event) { const current = (approvalAnswers[id] ??= Object.create(null))[question] ?? []; approvalAnswers[id][question] = (event.target as HTMLInputElement).checked ? [...new Set([...current, value])] : current.filter(item => item !== value); }
function otherAnswer(id: string, question: string, event: Event, multiSelect = false) { const value = (event.target as HTMLInputElement).value; (approvalOther[id] ??= Object.create(null))[question] = value; if (value.trim() && !multiSelect) (approvalAnswers[id] ??= Object.create(null))[question] = []; }
function incompleteAnswers(item: Extract<ChatItem, { type: 'approval' }>) { const payload = approvalPayloadAnswers(item.questions, approvalAnswers[item.id], approvalOther[item.id]); return item.questions.some(question => !Object.hasOwn(payload, question.id)); }
async function approve(item: Extract<ChatItem, { type: 'approval' }>, decision: ApprovalDecision) {
  if (isPreview.value || approvalBusy.value || item.resolved || !approvalChoices(item).includes(decision) || decision === 'accept' && incompleteAnswers(item)) return;
  const id = props.session.id; approvalBusy.value = item.id; error.value = '';
  const answers = decision === 'accept' ? approvalPayloadAnswers(item.questions, approvalAnswers[item.id], approvalOther[item.id]) : {};
  try { await request(`${sessionPath(id)}/conversation/approval`, json('POST', { request_id: item.id, decision, ...(Object.keys(answers).length ? { answers } : {}) })); if (props.session.id === id && mounted.value) { clearAnswers(item.id); await load(); } }
  catch (cause) { if (props.session.id === id && mounted.value) error.value = errorMessage(cause); }
  finally { if (props.session.id === id) approvalBusy.value = ''; }
}
async function configure() {
  if (!canConfigure.value || !endpointConfirm.value) return;
  await runAction(async id => {
    const payload = sessionConfigurationPayload(props.session, props.profiles, selectedProfile.value || null, true, { mode: mode.value, ready: view.value.ready, busy: view.value.turn === 'running', awaitingApproval: view.value.awaitingApproval }, { effort: selectedEffort.value });
    await request<Session>(`${sessionPath(id)}/configuration`, json('PATCH', payload));
    if (props.session.id === id && mounted.value) { endpointConfirm.value = false; selectedEffort.value = ''; notice.value = 'Endpoint changed. The next message starts a new native context and takes the conversation so far with it.'; emit('changed'); await load(); }
  });
}
function chooseEndpoint() { endpointConfirm.value = selectedProfile.value !== (props.session.endpoint_profile_id ?? '') || !!selectedEffort.value; }
/** Thinking depth applies in place, exactly like the model. Routing it through
 * the endpoint change would restart the bridge — and an official account refuses
 * a model or depth on that path at all, because those belong to its own client. */
async function applyEffort(level: string) {
  if (!canPickModel.value) return;
  await runAction(async session => {
    await request(`${sessionPath(session)}/conversation/model`, json('POST', { effort: level }));
  });
}
function syncCaret(event: Event) {
  const input = event.target as HTMLTextAreaElement;
  caret.value = input.selectionStart ?? input.value.length;
  if (!input.value.startsWith('/')) commandsDismissed.value = false;
}
function chooseCommand(command: string) {
  const query = commandQuery.value; if (!query) return;
  const next = applyCommand(draft.value.text, query, command);
  draft.value.text = next.text; commandsDismissed.value = false; commandIndex.value = 0;
  void nextTick(() => {
    const input = composerInput.value; if (!input) return;
    input.focus(); input.setSelectionRange(next.caret, next.caret); caret.value = next.caret;
  });
}
/** A key for the open suggestion list; whether the list took it. */
function listKeydown(event: KeyboardEvent, index: Ref<number>, count: number, choose: () => void, dismiss: () => void) {
  const action = suggestionKey(event);
  if (!action) return false;
  event.preventDefault();
  if (action === 'next' || action === 'previous') { index.value = moveHighlight(index.value, count, action === 'next' ? 1 : -1); return true; }
  event.stopPropagation();
  if (action === 'choose') choose(); else dismiss();
  return true;
}
function keydown(event: KeyboardEvent) {
  if (composing.value || event.isComposing || event.keyCode === 229) return;
  // Only one list is ever open: a command and a mention cannot both be under
  // the caret, so they never contend for the keys (chat-keyboard.ts).
  if (commandsOpen.value && listKeydown(event, commandIndex, commandMatches.value.length, () => chooseCommand(commandMatches.value[commandIndex.value] ?? commandMatches.value[0]), () => { commandsDismissed.value = true; })) return;
  if (mentionsOpen.value && listKeydown(event, mentionIndex, mentionHits.value.length, () => chooseMention(mentionHits.value[mentionIndex.value] ?? mentionHits.value[0]), () => { mentionsDismissed.value = true; })) return;
  if (answerFromKeys(event)) return;
  if (event.key !== 'Enter') return;
  // Shift+Enter keeps the native newline behavior; plain Enter and Ctrl/⌘
  // Enter submit. Repeated keydown is suppressed to avoid duplicate turns.
  // On a phone plain Enter stays a newline: several paragraphs are typed far
  // more often than a turn is meant to leave by accident, and the send button
  // is under the thumb. Ctrl/⌘+Enter from a hardware keyboard still sends.
  if (event.shiftKey || event.repeat) return;
  if (touchFirst && !event.ctrlKey && !event.metaKey) return;
  event.preventDefault(); event.stopPropagation();
  if (!sessionMenu.value?.open && !confirmEnd.value && !endpointConfirm.value) void send(event.altKey);
}
</script>

<template>
  <section :class="['chat-pane', { 'has-tab-menu': !!paneId, 'is-phone': mobile }]" :aria-label="t('Conversation with {provider}', {provider:providerLabel(session.provider)})" @keydown="paneKeydown">
    <ContextMenu v-if="timelineMenu" :x="timelineMenu.x" :y="timelineMenu.y" :label="t('Conversation actions')" @close="closeTimelineMenu"><button v-if="timelineMenu.text" role="menuitem" @click="copyFromTimeline(timelineMenu!.text)">{{ t('Copy selection') }}</button><button role="menuitem" @click="closeTimelineMenu(); scrollToLatest(true)">{{ t('Latest message') }}</button><button v-if="!isPreview" role="menuitem" @click="closeTimelineMenu(); openSearch()">{{ t('Search conversation') }} <kbd>{{ findKey }}</kbd></button><button role="menuitem" :disabled="loading" @click="closeTimelineMenu(); load()">{{ t('Refresh conversation') }}</button><template v-if="clearAvailable"><hr/><button role="menuitem" :disabled="actionBusy" :title="t('Runs this client\'s own /clear: the transcript and its context are dropped together.')" @click="clearContext">{{ t('Clear context') }}</button></template></ContextMenu>
    <Teleport v-if="!isPreview" to="body" :disabled="!paneId"><details :data-session-menu-pane="paneId" ref="sessionMenu" :style="tabMenuStyle" :class="['chat-menu','chat-floating-menu', { 'chat-tab-menu': !!paneId, 'is-phone': mobile }]" @toggle="positionMenu" @keydown.esc.stop.prevent="closeSessionMenu(true)"><summary :aria-label="t('Session actions')" :title="t('Session actions')"><Icon name="more" :size="17" /></summary><nav :style="tabMenuPanelStyle"><div class="chat-menu-status"><span class="chat-status-dot" :class="{working:turnBusy}" />{{ t(statusLabel,{provider:providerLabel(session.provider)}) }}</div><div class="chat-menu-session"><button v-if="mobile" @click="closeSessionMenu(); load()"><Icon name="refresh" :size="14" />{{ t('Refresh conversation') }}</button><slot name="session-actions" :close-menu="closeSessionMenu"><button @click="closeSessionMenu(); emit('environment',session.id)"><Icon name="settings" :size="14" />{{ t('Session environment') }}</button></slot></div><hr v-if="legacyAvailable||terminalReopenOffered||capabilities.agentTools" class="chat-menu-divider" /><button v-if="legacyAvailable" @click="closeSessionMenu(); emit('legacy')"><Icon name="terminal" :size="14" />{{ t('Open native client view') }}</button><button v-if="terminalReopenOffered" :disabled="actionBusy||!terminalReopenAvailable" :title="t(terminalReopenAvailable?'For interactive commands such as /config, which the conversation view cannot display':'Available once the conversation has started: send a first message')" @click="closeSessionMenu(); openInTerminal()"><Icon name="terminal" :size="14" />{{ t('Open this session in a terminal') }}</button><AgentToolsSwitch v-if="capabilities.agentTools" :session-id="session.id" /><hr v-if="running" class="chat-menu-divider" /><button v-if="running" class="chat-end-button" :disabled="actionBusy" @click="requestEnd"><Icon name="stop" :size="14" />{{ t('End session…') }}</button></nav></details></Teleport>
    <div v-if="!supported" class="chat-notice">{{ t('Structured conversation requires an updated backend. Your native session is unchanged.') }}<button v-if="legacyAvailable" class="chat-link" @click="emit('legacy')">{{ t('Open native client view') }}</button></div>
    <div v-if="error" class="chat-alert" role="alert">{{ error }}<button @click="load()">{{ t('Refresh conversation') }}</button></div>
    <div v-if="streamNotice&&!loading" class="chat-notice" role="status">{{ t(streamNotice) }}<button class="chat-link" @click="load()">{{ t('Reconnect view') }}</button></div>
    <div v-if="notice" class="chat-notice">{{ t(notice) }}</div>
    <div v-if="session.native_source_id" class="chat-notice">{{ t('Native history resumes in the client; earlier transcript is not yet displayed here.') }}</div>
    <div v-if="confirmEnd&&!isPreview" class="chat-end-confirm" role="alertdialog" :aria-label="t('End session')"><p>{{ t('End this session? Its background process will stop; history remains.') }}</p><div><button :disabled="actionBusy" @click="endSession">{{ t('End session') }}</button><button :disabled="actionBusy" @click="confirmEnd=false">{{ t('Keep connected') }}</button></div></div>
    <div v-if="searchOpen" class="chat-search" role="search"><Icon name="search" :size="14" class="chat-search-icon"/><input ref="searchInput" v-model="searchQuery" class="chat-search-input" type="text" enterkeyhint="search" autocomplete="off" spellcheck="false" maxlength="200" :placeholder="t('Search this conversation')" :aria-label="t('Search this conversation')" @keydown="searchKeydown" /><span v-if="searchCount" class="chat-search-count" role="status">{{ searchCount }}</span><span class="chat-search-divider" aria-hidden="true"/><button type="button" class="chat-search-button" :aria-label="t('Previous match')" :title="t('Previous match')" :disabled="!searchHits.length" @click="stepSearch(-1)"><Icon name="chevron" :size="14" class="chat-search-up"/></button><button type="button" class="chat-search-button" :aria-label="t('Next match')" :title="t('Next match')" :disabled="!searchHits.length" @click="stepSearch(1)"><Icon name="chevron" :size="14" class="chat-search-down"/></button><button type="button" class="chat-search-button" :aria-label="t('Close search')" :title="t('Close search')" @click="closeSearch"><Icon name="close" :size="13"/></button><p v-if="searchNotice" class="chat-search-notice" role="status">{{ t(searchNotice) }}</p></div>
    <div ref="viewport" tabindex="-1" :class="['chat-timeline',{'is-empty':!view.items.length}]" @wheel.passive="timelineWheel" @touchstart.passive="timelineTouchStart" @touchmove.passive="timelineTouchMove" @keydown="timelineKeydown" @pointerdown="byPerson" role="log" :aria-label="t('Conversation messages')" aria-live="polite" :aria-busy="view.turn==='running'" @contextmenu="openTimelineMenu" @scroll="atBottom">
      <p v-if="truncated" class="chat-history-notice">{{ t('Earlier display history was trimmed. Native history remains managed by the official client.') }}</p>
      <div v-if="!view.items.length" class="chat-welcome"><span class="chat-welcome-mark"><ProviderIcon :provider="session.provider" :size="32" /></span><h3>{{ t('What shall we build?') }}</h3><p>{{ t('A real conversation with your native agent, with room for tools, changes and your next idea.') }}</p><span class="chat-context-chip" :title="activeProfile?(activeProfile.native_config?t('Uses the sign-in and settings in {path}',{path:activeProfile.native_config.config_dir}):undefined):t('A fresh, empty client home: no host sign-in or settings, so it may ask you to log in.')"><ProviderIcon :provider="session.provider" :size="12"/>{{ endpointName }}</span><Transition name="chat-boot" :duration="240"><p v-if="booting" class="chat-boot" role="status"><span class="chat-boot-bar"><i/></span>{{ t('Starting {provider}… models and commands arrive with it.',{provider:providerLabel(session.provider)}) }}</p></Transition><div v-if="startersAvailable" class="chat-starters"><button v-for="(starter,index) in STARTERS" :key="starter.title" type="button" :style="{'--starter-delay':index*60+'ms'}" @click="useStarter(starter.prompt)"><span class="chat-starter-icon" :data-tone="starter.icon"><Icon :name="starter.icon" :size="16"/></span><span><strong>{{ t(starter.title) }}</strong><small>{{ t(starter.note) }}</small></span></button></div><p v-if="chatNote" class="chat-trust-note">{{ t(chatNote) }}</p></div>
      <div v-if="older!==undefined" ref="olderSentinel" class="chat-history-older" role="status">{{ loadingOlder ? t('Loading earlier messages…') : '' }}</div>
      <VirtualRows ref="rowsList" v-slot="{ row: item }" :rows="rows.rows" :keys="rows.keys" :scroll-element="viewport" :scroll-margin="rowsMargin" :estimate="rowHeight" :measured="rememberRowHeight" :memo="[running, slowNow, locale]" @resize="onRowsResize" @range="onRowsRange">
        <article v-if="item.type==='message'" :class="['chat-message',item.role]"><div class="chat-message-label"><ProviderIcon v-if="item.role==='assistant'" :provider="session.provider" :size="15" /><span>{{ item.role==='user'?t('You'):providerLabel(session.provider) }}</span><em v-if="item.steered" class="chat-steered" :title="t('Written while the agent worked, and read inside that turn')">{{ t('Steer') }}</em></div><div v-if="item.role==='user'&&attachedImagePaths(item.text).length" class="chat-attached-images"><img v-for="path in attachedImagePaths(item.text)" :key="path" :src="workspaceImage(path)" :alt="path" :title="path" loading="lazy" @click="openLightbox(workspaceImage(path), path)" /></div><MarkdownContent :text="item.text" :image-url="workspaceImage" :host-image-url="hostImage" :open-file="openReference" /><div class="chat-message-foot"><div class="chat-message-actions"><CopyButton :text="item.text" label="Copy message" compact /></div><span v-if="item.role==='assistant'&&item.usage" class="chat-turn-usage" :title="turnUsageTitle(item.usage)">{{ turnUsageLine(item.usage) }}</span></div></article>
        <component :is="item.type==='tool_run'&&item.tools.length>1?'details':'div'" v-else-if="item.type==='tool_run'" :class="item.tools.length>1?['chat-tool-run',toolRunStatus(item)]:'chat-tool-solo'"><summary v-if="item.tools.length>1"><span :class="['tool-indicator',toolRunStatus(item)]">{{ toolIndicator({ status: toolRunStatus(item) }) }}</span><strong>{{ t('{count} tool calls',{count:item.tools.length}) }}</strong><small>{{ toolRunStatus(item)==='running' ? runningLine(item) : toolRunNames(item) }}</small><Icon class="chat-tool-chevron" name="chevron" :size="14" /></summary><details v-for="tool in item.tools" :key="tool.id" class="chat-tool" :data-tool-id="tool.id"><summary><span :class="['tool-indicator',tool.status]">{{ toolIndicator(tool) }}</span><strong>{{ tool.name }}<template v-if="tool.label"> · {{ tool.label }}</template></strong><small>{{ tool.activity && tool.status==='running' ? lastLine(tool.activity) : t(tool.status==='running'?(running&&!tool.ended?'Working…':'Session ended'):tool.status==='failed'?'Failed':'Completed') }}</small><span v-if="tool.stats" class="chat-tool-stats">{{ statsLine(tool.stats) }}</span><span v-else-if="sizeWorthShowing(tool)" class="chat-tool-stats" :title="t('Its result added about {count} tokens to the context', { count: tool.result_tokens! })">≈ {{ t('{count} tokens', { count: compactCount(tool.result_tokens!) }) }}</span><em v-if="quietLine(tool)" class="chat-tool-stalled">{{ quietLine(tool) }}</em><Icon class="chat-tool-chevron" name="chevron" :size="14" /></summary><pre v-if="tool.activity" class="tool-activity">{{ tool.activity }}</pre><div v-if="tool.children?.length" class="chat-tool-children"><details v-for="entry in descendants(tool)" :key="entry.tool.id" class="chat-tool-nested" :style="{'--depth':entry.depth}" :data-tool-id="entry.tool.id"><summary><span :class="['tool-indicator',entry.tool.status]">{{ toolIndicator(entry.tool) }}</span><strong>{{ entry.tool.name }}<template v-if="entry.tool.label"> · {{ entry.tool.label }}</template></strong><small>{{ entry.tool.activity && entry.tool.status==='running' ? lastLine(entry.tool.activity) : t(entry.tool.status==='running'?(running&&!entry.tool.ended?'Working…':'Session ended'):entry.tool.status==='failed'?'Failed':'Completed') }}</small><span v-if="entry.tool.stats" class="chat-tool-stats">{{ statsLine(entry.tool.stats) }}</span><span v-else-if="sizeWorthShowing(entry.tool)" class="chat-tool-stats" :title="t('Its result added about {count} tokens to the context', { count: entry.tool.result_tokens! })">≈ {{ t('{count} tokens', { count: compactCount(entry.tool.result_tokens!) }) }}</span><em v-if="quietLine(entry.tool)" class="chat-tool-stalled">{{ quietLine(entry.tool) }}</em></summary><pre v-if="entry.tool.activity" class="tool-activity">{{ entry.tool.activity }}</pre><pre v-if="entry.tool.text">{{ entry.tool.text }}</pre></details></div><pre v-if="tool.text">{{ tool.text }}</pre><p v-else-if="!tool.activity&&!tool.children?.length">{{ t('The client did not provide tool output.') }}</p></details></component>
        <p v-else-if="item.type==='approval'" :class="['chat-approval-record',{pending:!item.resolved}]"><Icon :name="item.resolved?'check':'shield'" :size="13"/><span>{{ item.title }}</span><small>{{ t(item.resolved?'Resolved':'Waiting for you below') }}</small></p>
        <div v-else-if="item.type==='configuration'" class="chat-boundary"><span>{{ t('New native context') }} · {{ item.profile_name }}</span><p>{{ t(item.text) }}</p></div>
        <p v-else-if="item.type==='error'" class="chat-alert" role="alert">{{ item.text }}</p>
      </VirtualRows>
      <div v-if="view.turn==='running'" class="chat-working"><ProviderIcon class="chat-working-mark" :provider="session.provider" :size="15"/><span class="chat-working-label">{{ t('Agent is working') }}</span></div>
      <p v-if="draft.pending" class="chat-delivery" role="status">{{ t(draft.notice??(draft.pending.state==='sending'?'Sending your message…':'Waiting for delivery confirmation.')) }}<button v-if="draft.pending.state==='unknown'" class="chat-link" @click="load()">{{ t('Refresh conversation') }}</button><button v-if="draft.pending.state==='unknown'&&!isPreview" class="chat-link" :disabled="loading||actionBusy||streamState!=='connected'" @click="retryDelivery">{{ t('Retry the same message request') }}</button></p>
      <div v-if="supported&&!loading&&mode!=='structured'" class="chat-native-note"><p>{{ t(running?'This session is running in the native client. End it there before enabling chat; it will not be taken over automatically.':'Enable chat explicitly to continue this native session in a conversation view.') }}</p><button class="chat-primary" :disabled="isPreview||actionBusy||running" @click="explicitOpen">{{ t('Enable conversation view') }}</button><button v-if="legacyAvailable" class="chat-link" @click="emit('legacy')">{{ t('Open native client view') }}</button></div>
    </div>
    <section v-if="dockAgents.length" ref="dock" :class="['subagent-dock',{collapsed:!dockOpen}]" :aria-label="t('Subagents')"><header><button type="button" class="subagent-dock-head" :aria-expanded="dockOpen" @click="dockOpen=!dockOpen"><span :class="['subagent-dock-dot',{idle:!dockRunning}]" /><strong>{{ dockHeading }}</strong><small v-if="!dockOpen&&dockAgents[0]!.activity">{{ lastLine(dockAgents[0]!.activity!) }}</small><Icon class="subagent-dock-chevron" name="chevron" :size="14" /></button><button v-if="dockAgents.some(agent=>agent.status!=='running')" type="button" class="subagent-dock-clear" @click="clearFinished">{{ t('Clear finished') }}</button></header><ul v-if="dockOpen"><li v-for="agent in dockAgents" :key="agent.id" :class="['subagent-item',agent.status,{open:expandedAgent===agent.id}]"><div class="subagent-row"><button type="button" class="subagent-toggle" :aria-expanded="expandedAgent===agent.id" @click="expandedAgent=expandedAgent===agent.id?null:agent.id"><span :class="['tool-indicator',agent.status]">{{ toolIndicator(agent) }}</span><strong><Icon v-if="!SUBAGENT_TOOLS.has(agent.name)" class="subagent-kind" name="terminal" :size="12" />{{ agent.label || agent.name }}</strong><small>{{ agent.activity ? lastLine(agent.activity) : t(agent.status==='running'?'Working…':agent.status==='failed'?'Failed':'Completed') }}</small><span v-if="agentStats(agent)" class="chat-tool-stats">{{ statsLine(agentStats(agent)!) }}</span><em v-if="quietLine(agent)" class="chat-tool-stalled">{{ quietLine(agent) }}</em><Icon class="subagent-chevron" name="chevron" :size="14" /></button><button v-if="agent.status!=='running'" type="button" class="subagent-dismiss" :aria-label="t('Dismiss')" :title="t('Dismiss')" @click="dismissAgent(agent.id)">×</button></div><div v-if="expandedAgent===agent.id" class="subagent-preview"><pre v-if="agent.activity" class="tool-activity subagent-log" @scroll="logScrolled">{{ agent.activity }}</pre><div v-if="agent.children?.length" class="chat-tool-children"><details v-for="entry in descendants(agent)" :key="entry.tool.id" class="chat-tool-nested" :style="{'--depth':entry.depth}"><summary><span :class="['tool-indicator',entry.tool.status]">{{ toolIndicator(entry.tool) }}</span><strong>{{ entry.tool.name }}<template v-if="entry.tool.label"> · {{ entry.tool.label }}</template></strong><small>{{ entry.tool.activity && entry.tool.status==='running' ? lastLine(entry.tool.activity) : t(entry.tool.status==='running'?(entry.tool.ended?'Session ended':'Working…'):entry.tool.status==='failed'?'Failed':'Completed') }}</small><span v-if="agentStats(entry.tool)" class="chat-tool-stats">{{ statsLine(agentStats(entry.tool)!) }}</span></summary><pre v-if="entry.tool.activity" class="tool-activity">{{ entry.tool.activity }}</pre><pre v-if="entry.tool.text">{{ entry.tool.text }}</pre></details></div><template v-if="!agent.activity&&!agent.children?.length"><pre v-if="agent.text" class="subagent-log">{{ agent.text }}</pre><p v-else class="subagent-empty">{{ t('Nothing reported yet.') }}</p></template></div></li></ul></section>
    <button v-if="!followBottom" class="chat-latest" :style="latestBottom!==undefined?{bottom:latestBottom+'px'}:undefined" @click="scrollToLatest(true)">↓ {{ t('Latest message') }}</button>
    <BottomSheet v-if="mobile && optionsOpen" :title="t('Session settings')" @opened="chipsInSheet=true" @close="optionsClosed"><div v-if="error" class="chat-alert chat-options-alert" role="alert">{{ error }}</div><div v-else-if="notice" class="chat-notice chat-options-alert" role="status">{{ t(notice) }}</div><div :id="optionsTarget" class="chat-options-body" /></BottomSheet>
    <section v-if="dockedApproval" class="chat-approval-dock" :aria-label="t('Waiting for you')">
      <div v-if="pendingApprovals.length>1" class="chat-approval-pager"><span>{{ t('{index} of {count} waiting for you', { index: dockedIndex+1, count: pendingApprovals.length }) }}</span><button type="button" class="icon-button" :disabled="dockedIndex===0" :aria-label="t('Previous request')" @click="approvalIndex=dockedIndex-1"><Icon name="chevron" :size="14" class="pager-back"/></button><button type="button" class="icon-button" :disabled="dockedIndex>=pendingApprovals.length-1" :aria-label="t('Next request')" @click="approvalIndex=dockedIndex+1"><Icon name="chevron" :size="14"/></button></div>
        <article v-for="item in [dockedApproval]" :key="item.id" :class="['chat-approval',item.questions.length?'is-question':'is-permission',{resolved:item.resolved,folded:foldedApprovals.has(item.id)}]"><header class="chat-approval-head"><span class="chat-approval-icon"><Icon :name="item.resolved?'check':item.questions.length?'spark':'shield'" :size="15" /></span><div class="chat-approval-heading"><small>{{ t(item.questions.length?'Question from the agent':'Permission request') }}</small><strong>{{ item.title }}</strong></div><span v-if="item.resolved" class="chat-approval-state">{{ t('Resolved') }}</span><button v-else type="button" class="icon-button chat-approval-fold" :aria-expanded="!foldedApprovals.has(item.id)" :aria-label="t(foldedApprovals.has(item.id)?'Expand':'Collapse')" :title="t(foldedApprovals.has(item.id)?'Expand':'Collapse')" @click="toggleApprovalFold(item.id)"><Icon name="chevron" :size="14" /></button></header><div class="chat-approval-body"><template v-if="approvalBody(item.text)"><p v-if="approvalBody(item.text)!.reason">{{ approvalBody(item.text)!.reason }}</p></template><MarkdownContent v-else :text="item.text" :open-file="openReference" /><div v-if="item.scope" class="chat-approval-scope"><span v-if="item.scope.access" :class="['scope-access', item.scope.access]">{{ t(ACCESS_LABEL[item.scope.access]!) }}</span><template v-if="item.scope.outside?.length"><strong class="scope-outside"><Icon name="info" :size="13" />{{ t('Outside the workspace') }}</strong><code v-for="path in item.scope.outside" :key="path" class="scope-path">{{ path }}</code></template><pre v-if="item.scope.command" class="scope-command">{{ item.scope.command }}</pre></div><details v-if="approvalBody(item.text)?.detail" class="chat-approval-detail"><summary>{{ t('Details') }}</summary><pre>{{ approvalBody(item.text)!.detail }}</pre></details></div><template v-if="!item.resolved">
          <fieldset v-for="question in item.questions" :key="question.id" class="chat-question" :disabled="isPreview||!!approvalBusy"><legend><span v-if="question.header" class="chat-question-tag">{{ question.header }}</span><span :id="'question-'+session.id+'-'+item.id+'-'+question.id">{{ question.question }}</span></legend>
            <div v-if="question.options.length&&!question.isSecret" class="chat-answers" :role="question.multiSelect?'group':'radiogroup'" :aria-labelledby="'question-'+session.id+'-'+item.id+'-'+question.id"><label v-for="option in question.options" :key="option.label" :class="['chat-answer',{selected:approvalAnswers[item.id]?.[question.id]?.includes(option.label)}]"><input :type="question.multiSelect?'checkbox':'radio'" :name="'answer-'+session.id+'-'+item.id+'-'+question.id" :value="option.label" :checked="approvalAnswers[item.id]?.[question.id]?.includes(option.label)??false" @change="question.multiSelect?toggleAnswer(item.id,question.id,option.label,$event):answer(item.id,question.id,$event)" /><span class="chat-answer-marker" aria-hidden="true"><Icon v-if="question.multiSelect" name="check" :size="11" /></span><span class="chat-answer-copy"><strong>{{ option.label }}</strong><small v-if="option.description">{{ option.description }}</small></span></label><label v-if="question.isOther" :class="['chat-answer','chat-answer-other',{selected:!!approvalOther[item.id]?.[question.id]?.trim()}]"><span class="chat-answer-marker" aria-hidden="true" /><input type="text" autocomplete="off" :placeholder="t('Other answer')" :aria-label="t('Other answer')" :value="approvalOther[item.id]?.[question.id]??''" @input="otherAnswer(item.id,question.id,$event,question.multiSelect)" /></label></div>
            <template v-else><input class="chat-question-input" :aria-labelledby="'question-'+session.id+'-'+item.id+'-'+question.id" :type="question.isSecret?'password':'text'" :list="!question.isSecret&&question.options.length?'choices-'+session.id+'-'+item.id+'-'+question.id:undefined" autocomplete="off" :placeholder="t('Type your answer')" :value="approvalAnswers[item.id]?.[question.id]?.[0]??''" @input="answer(item.id,question.id,$event)" /><datalist v-if="!question.isSecret&&question.options.length" :id="'choices-'+session.id+'-'+item.id+'-'+question.id"><option v-for="option in question.options" :key="option.label" :value="option.label">{{ option.description }}</option></datalist></template>
          </fieldset>
          <footer v-if="approvalChoices(item).length" class="chat-approval-actions"><button v-for="choice in [...approvalChoices(item)].reverse()" :key="choice" type="button" :class="choice==='accept'?'allow':choice==='accept_session'?'allow-session':choice==='decline'?'deny':'cancel'" :disabled="isPreview||!!approvalBusy||streamState!=='connected'||(choice==='accept'&&incompleteAnswers(item))" @click="approve(item,choice)">{{ t(choice==='accept'&&item.questions.length?'Submit answers':DECISION_LABEL[choice]) }}<kbd v-if="choice==='accept'&&!mobile" class="chat-approval-key">{{ acceptKey }}</kbd><kbd v-else-if="choice==='decline'&&!mobile" class="chat-approval-key">{{ declineKey }}</kbd></button></footer><p v-else class="chat-approval-note">{{ t('This native approval needs the original client. No permission was granted automatically.') }}</p>
        </template></article>
    </section>
    <form :class="['chat-composer', { 'is-phone': mobile }]" @submit.prevent="send()">
      <div v-if="branchConfirm" class="chat-endpoint-confirm"><strong>{{ t('Move this session to {branch}?', { branch: branchConfirm }) }}</strong><p>{{ t('It restarts in that branch\'s worktree with a new native context; the conversation so far goes with the next message.') }}</p><div><button type="button" :disabled="actionBusy" @click="moveToBranch(branchConfirm)">{{ t('Move to {branch}', { branch: branchConfirm }) }}</button><button type="button" @click="branchConfirm=''">{{ t('Cancel') }}</button></div></div><div v-if="handoffTarget" class="chat-endpoint-confirm chat-handoff-confirm"><strong>{{ t('Continue this conversation in {provider}?', { provider: providerLabel(handoffTarget.provider) }) }}</strong><p>{{ t('A new {provider} session opens with this conversation written into its message box. Read it, add what to do next, then send. This session stays as it is; tool output and files it opened are not carried over.', { provider: providerLabel(handoffTarget.provider) }) }}</p><div><button type="button" :disabled="actionBusy" @click="confirmHandoff">{{ t('Open in {provider}', { provider: providerLabel(handoffTarget.provider) }) }}</button><button type="button" @click="handoffTarget=undefined">{{ t('Cancel') }}</button></div></div><div v-if="launchFallback" class="chat-endpoint-confirm"><strong>{{ t('{model} could not be switched to mid-conversation', { model: launchFallbackName }) }}</strong><p>{{ t('This endpoint refuses the availability check the client makes when switching models live. Starting the session on this model works instead, but begins a new native context: the conversation so far goes with the next message, without tool output.') }}</p><div><button type="button" :disabled="!canConfigure" @click="applyModelAtLaunch">{{ t('Start a new context on this model') }}</button><button type="button" @click="launchFallback=''">{{ t('Cancel') }}</button></div></div><div v-if="endpointConfirm" class="chat-endpoint-confirm"><strong>{{ t('Apply this configuration change?') }}<span v-if="selectedProfile!==(session.endpoint_profile_id??'')" class="chat-endpoint-target">→ {{ selectedProfile ? profiles.find(p=>p.id===selectedProfile)?.name : t('Native · isolated configuration') }}<BalanceBadge :profile-id="selectedProfile"/></span></strong><p>{{ t('Only an idle session can switch. Its current bridge will close, and the next message starts a new native context with the conversation so far written ahead of it. Tool output is not carried over.') }}</p><div><button type="button" :disabled="!canConfigure" @click="configure">{{ t('Confirm endpoint change') }}</button><button type="button" @click="endpointConfirm=false;selectedProfile=session.endpoint_profile_id??'';selectedEffort=''">{{ t('Cancel') }}</button></div></div>
      <div v-if="mentionsOpen" class="chat-commands chat-mentions" role="listbox" :aria-label="t('Workspace files')"><button v-for="(hit,index) in mentionHits" :key="hit.path" type="button" role="option" :aria-selected="index===mentionIndex" :class="{highlighted:index===mentionIndex}" @mousedown.prevent="chooseMention(hit)" @mouseenter="mentionIndex=index"><strong>{{ hit.name }}</strong><small>{{ hit.path }}</small></button><small>{{ t('Files in this workspace · Enter or Tab to insert') }}</small></div>
      <div v-if="commandsOpen" class="chat-commands" role="listbox" :aria-label="t('Native client commands')"><button v-for="(command,index) in commandMatches" :key="command" type="button" role="option" :aria-selected="index===commandIndex" :class="{highlighted:index===commandIndex}" @mousedown.prevent="chooseCommand(command)" @mouseenter="commandIndex=index">/{{ command }}</button><small>{{ t('From this client · Enter or Tab to complete') }}</small></div>
      <div v-if="queued.length" class="chat-queue" role="list" :aria-label="t('Queued messages')"><div v-for="(item,index) in queued" :key="item.id" class="chat-queued" role="listitem"><span class="chat-queued-mark">{{ index===0 ? t('Next') : index+1 }}</span><p :title="item.text">{{ item.text.trim() || item.attachments?.map(file=>file.name).join(', ') }}<small v-if="item.text.trim()&&item.attachments?.length" class="chat-queued-files"> · {{ t('{count} attached', { count: item.attachments.length }) }}</small></p><button v-if="view.turn==='running'" type="button" :title="t('Hand this to the running turn; the agent reads it at its next step')" :disabled="steering" @click="steerQueued(item.id)">{{ t('Steer') }}</button><button type="button" :title="t('Interrupt the agent and send this now')" @click="sendQueuedNow(item.id)">{{ t('Send now') }}</button><button type="button" :title="t('Move back into the message box')" @click="editQueued(item.id)">{{ t('Edit') }}</button><button type="button" class="chat-queued-remove" :aria-label="t('Remove from queue')" @click="messageQueue.remove(session.id,item.id)">×</button></div><small>{{ t('Sent in order when the agent finishes.') }}</small></div>
      <textarea ref="composerInput" data-pane-input v-model="draft.text" :aria-label="t('Message your agent')" :placeholder="booting?t('{provider} is starting — you can already type…',{provider:providerLabel(session.provider)}):view.turn==='running'?t(mobile?'Steer the agent…':'Agent is working — Enter steers it, ⌥/Alt+Enter queues for after'):mobile?t('Message {provider}…',{provider:providerLabel(session.provider)}):t('Ask your agent to build, explore, or fix something…')" rows="2" :disabled="!supported||!structuredSession||!!draft.pending" @focus="wake" @paste="pasteFiles" @dragover.prevent @drop="dropFiles" @compositionstart="composing=true" @compositionend="composing=false" @keydown="keydown" @keyup="syncCaret" @click="syncCaret" @input="syncCaret" />
      <ul v-if="attachments.length" class="chat-attachments" :aria-label="t('Attached files')"><li v-for="file in attachments" :key="file.path"><Icon name="file" :size="13"/><span class="chat-attachment-name" :title="file.path">{{ file.name }}</span><small>{{ formatBytes(file.bytes) }}</small><button type="button" :aria-label="t('Remove {name}',{name:file.name})" @click="removeAttachment(file.path)"><Icon name="close" :size="12"/></button></li></ul>
      <div class="chat-composer-controls"><input ref="fileInput" class="sr-only" type="file" multiple :disabled="isPreview||uploading" @change="pickAttachments"/><button type="button" class="chat-attach" :aria-label="t(uploading?'Uploading…':'Attach files')" :title="t('Attach files to this workspace')" :disabled="isPreview||uploading||!supported" @click="fileInput?.click()"><Icon name="plus" :size="18"/></button><button v-if="mobile" type="button" :class="['chat-options', { danger: permissionDanger }]" :aria-label="t('Session settings')" @click="optionsOpen=true"><Icon name="settings" :size="17"/><span>{{ optionsSummary }}</span><Icon v-if="permissionDanger" class="chat-options-danger" name="shield" :size="14" :title="t('Tools run without asking')" /><Icon name="chevron" :size="12" class="chat-options-caret"/></button><Teleport :to="chipsInSheet ? '#'+optionsTarget : 'body'" :disabled="!chipsInSheet"><div :class="['chat-chip-strip', { 'chip-rows': chipsInSheet }]"><ChipMenu v-if="repoState" ref="branchMenu" class="chat-branch" :caption="t('Branch')" :label="sessionBranch || t('Detached HEAD')" :active="!!session.checkout_path" :disabled="actionBusy||turnBusy" :title="session.checkout_path ? t('Works in worktree {path}', { path: session.checkout_path }) : t('Works in the workspace directory')"><template #mark><svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path d="M5 3v7M5 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4Zm0-7a2 2 0 1 0 0-.01M11 5a2 2 0 1 0 0-.01M11 7c0 2-2 3-6 3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg></template><div class="chat-branch-body"><header>{{ t('Branch for this session') }}</header><p>{{ t('Moves only this session, into that branch\'s own worktree. Other sessions keep their branches.') }}</p><nav><button v-for="name in branchChoices" :key="name" type="button" :class="{selected:name===sessionBranch}" @click="pickBranch(name)"><span>{{ name }}</span><small v-if="name===sessionBranch">{{ t('Current') }}</small><small v-else-if="name===repoState.current">{{ t('Workspace directory') }}</small></button></nav><form @submit.prevent="pickBranch(branchTarget)"><input v-model="branchTarget" :placeholder="t('New branch name')" maxlength="200" spellcheck="false" autocomplete="off" /><button type="submit" :disabled="!branchTarget.trim()">{{ t('Create') }}</button></form></div></ChipMenu><ChipMenu ref="endpointMenu" class="chat-endpoint" :caption="t('Endpoint profile')" :label="endpointName" :disabled="!canOpenEndpoint" :title="t('Endpoint profile')"><template #mark><ProviderIcon :provider="session.provider" :size="14"/></template><template #badge><BalanceBadge v-if="!isPreview" :profile-id="session.endpoint_profile_id" compact/></template><nav :aria-label="t('Endpoint profile')"><p v-if="!canConfigure" class="chat-menu-note">{{ t('Wait for the current turn and approvals before changing endpoints.') }}</p><button type="button" :class="{selected:selectedProfile===''}" :disabled="!canConfigure" @click="pickEndpoint('')"><strong>{{ t('Native · isolated configuration') }}</strong><small>{{ t('A fresh, empty client home: no host sign-in or settings, so it may ask you to log in.') }}</small></button><button v-for="profile in matchingProfiles" :key="profile.id" type="button" :class="{selected:selectedProfile===profile.id}" :disabled="!canConfigure" @click="pickEndpoint(profile.id)"><strong><span>{{ profile.name }}</span><BalanceBadge :profile-id="profile.id"/></strong><small>{{ profileNote(profile, t) }}</small></button><button v-if="session.endpoint_profile_id&&!matchingProfiles.some(p=>p.id===session.endpoint_profile_id)" type="button" class="selected" disabled>{{ endpointName }}</button><template v-if="handoffChoices.length"><p class="chat-menu-section">{{ t('Switch to another client') }}<small>{{ t('Opens a new session with this conversation in its message box; this one stays as it is.') }}</small></p><button v-for="choice in handoffChoices" :key="choice.provider+':'+(choice.profileId??'')" type="button" class="chat-handoff-choice" @click="pickHandoff(choice)"><strong><ProviderIcon :provider="choice.provider" :size="12"/><span>{{ providerLabel(choice.provider) }} · {{ choice.name }}</span><BalanceBadge :profile-id="choice.profileId"/></strong><small>{{ choice.note }}</small></button></template><button v-if="!isPreview&&balanceFor(session.endpoint_profile_id)" type="button" class="chat-endpoint-manage" :disabled="balances.busy.includes(session.endpoint_profile_id!)" @click="refreshBalance(session.endpoint_profile_id!)"><Icon name="refresh" :size="13" />{{ t(balances.busy.includes(session.endpoint_profile_id!)?'Querying balance…':'Query this endpoint\'s balance again') }}</button><button type="button" class="chat-endpoint-manage" @click="endpointMenu?.close(true); emit('profiles')"><Icon name="account" :size="13" />{{ t('Manage endpoint profiles') }}</button></nav></ChipMenu><span v-if="booting" class="chat-boot-chip" role="status"><ProviderIcon class="chat-working-mark" :provider="session.provider" :size="12"/>{{ t('Starting…') }}</span><ChipMenu v-if="canTuneModel&&(models.length||currentModel)" ref="modelMenu" class="chat-model" :caption="t('Model')" :label="modelName" :active="!modelDefault" :disabled="!canPickModel" :title="canPickModel?t('Model'):t('Model: {model}',{model:modelName})"><template #mark><Icon name="spark" :size="12"/></template><nav :aria-label="t('Model')"><button v-for="entry in visibleModels" :key="entry.id" type="button" :class="{selected:entry.id===currentModel}" @click="pickModel(entry.id)"><strong>{{ entry.name }}</strong><code v-if="entry.id.toLocaleLowerCase()!==entry.name.toLocaleLowerCase()" class="chat-model-id">{{ entry.id }}</code><small v-if="entry.description">{{ describeModel(entry.description, t) }}</small></button><button v-if="customPicked&&!models.some(entry=>entry.id===customPicked)" type="button" :class="{selected:customPicked===currentModel}" @click="pickModel(customPicked)"><strong>{{ customPicked }}</strong><small>{{ t('Entered by hand') }}</small></button></nav><div class="chat-model-custom"><input v-model="customModel" type="text" autocomplete="off" spellcheck="false" maxlength="128" :placeholder="t('Find a model, or enter an ID')" :aria-label="t('Other model ID')" @keydown.enter.prevent.stop="pickCustomModel"/><button type="button" :disabled="!customModel.trim()" @click="pickCustomModel">{{ t('Use') }}</button></div><p class="chat-model-hint">{{ t(profileCatalog.length ? 'From the {provider} client on the host and from this endpoint. Type to find a model, or enter an ID.' : 'This list comes from the {provider} client on the host. Upgrade it there to see newer models, or enter an ID.',{provider:providerLabel(session.provider)}) }}</p></ChipMenu><ChipMenu v-if="effortChoices.length" class="chat-effort" :caption="t('Thinking depth')" :class="{vivid:effortVivid}" :label="effortName" :active="!!liveEffort" :disabled="!canConfigure" :title="canConfigure?t('Thinking depth'):t('Wait for the current turn and approvals to finish before changing this.')"><template #mark><Icon :name="effortVivid?'spark':'gauge'" :size="12"/></template><div class="chat-effort-body"><p v-if="manualEffort" class="chat-effort-note">{{ t('Actual effort support depends on the endpoint and model.') }}</p><header><strong>{{ t('Thinking depth') }}</strong><em>{{ effortSetting }}</em></header><div class="chat-effort-slider" :style="{'--effort-fill':effortFill}"><span class="chat-effort-track"><i v-for="(stop,index) in effortStops" :key="stop||'default'" :style="{left:effortStops.length<2?'50%':index/(effortStops.length-1)*100+'%'}"/></span><span class="chat-effort-fill"/><span class="chat-effort-thumb"/><input type="range" min="0" :max="effortStops.length-1" step="1" :value="effortIndex" :disabled="!canPickModel" :aria-label="t('Thinking depth')" :aria-valuetext="effortName" @input="dragEffort" @change="commitEffort"/></div><footer><span>{{ t('Faster') }}</span><span>{{ t('Smarter') }}</span></footer></div></ChipMenu><ChipMenu v-if="permissionChoices.length" ref="permissionMenu" class="chat-permission" :caption="t('How tools are approved')" :class="{danger:permissionDanger}" :label="permissionName" :tone="permissionDanger?'danger':undefined" :disabled="!canConfigure" :title="canConfigure?t('How tools are approved'):t('Wait for the current turn and approvals to finish before changing this.')"><template #mark><Icon name="shield" :size="12"/></template><div class="chat-permission-body"><header>{{ t('How tools are approved') }}</header><nav :aria-label="t('How tools are approved')"><button v-for="mode in permissionChoices" :key="mode" type="button" :class="{selected:mode===livePermission,danger:mode==='danger'}" @click="pickPermission(mode)"><span><strong>{{ t(PERMISSION_LABELS[mode] ?? mode) }}</strong><Icon v-if="mode===livePermission" name="check" :size="13"/></span><small v-if="PERMISSION_NOTES[mode]">{{ t(PERMISSION_NOTES[mode]) }}</small></button></nav></div></ChipMenu></div></Teleport><template v-if="view.turn==='running'&&canQueue"><button type="button" class="chat-send chat-steer-send" :aria-label="t('Steer the running turn')" :title="t('Steer the running turn (Enter)')" :disabled="steering" @click="steerDraft">↪</button><ChipMenu ref="steerMenu" class="chat-steer-more" :label="''" :title="t('Other ways to send')"><nav :aria-label="t('Other ways to send')"><button type="button" @click="steerMenu?.close(true);steerDraft()"><strong>{{ t('Steer') }}</strong><small>{{ t('The agent reads it at its next step, without stopping. Enter') }}</small></button><button type="button" @click="steerMenu?.close(true);queueDraft()"><strong>{{ t('After this turn') }}</strong><small>{{ t('Sent as its own turn when this one ends. ⌥/Alt+Enter') }}</small></button><button type="button" class="danger" @click="interruptWithDraft"><strong>{{ t('Interrupt and send') }}</strong><small>{{ t('Stops the current turn; its unfinished work stops too.') }}</small></button></nav></ChipMenu></template><button v-if="view.turn==='running'" type="button" class="chat-send chat-stop" :aria-label="t('Cancel turn')" :disabled="isPreview||actionBusy" @click="interrupt">■</button><button v-else class="chat-send" type="submit" :aria-label="t('Send message')" :disabled="!canSend">↑</button></div>
      <footer><span :title="t('Ctrl / ⌘ Enter also sends. Esc closes menus first; in the message box it cancels the active reply, not the session.')">{{ t(view.turn==='running' ? 'Enter steers · ⌥/Alt+Enter after this turn · Shift+Enter for a new line' : 'Enter to send · Shift+Enter for a new line · Esc to dismiss or cancel') }}</span><span class="chat-usage"><ContextRing :usage="ringUsage"/><span v-if="view.usage.context_window===undefined&&view.usage.context_tokens!==undefined">{{ t('{count} context tokens',{count:view.usage.context_tokens}) }}</span><span v-else-if="view.usage.context_window===undefined&&view.usage.input_tokens!==undefined">{{ t('{count} input tokens',{count:view.usage.input_tokens}) }}</span></span></footer>
    </form>
  </section>
</template>

<style scoped>
.chat-pane{height:100%;min-height:0;min-width:0;display:flex;flex-direction:column;position:relative;background:var(--surface);color:var(--ink);container-type:inline-size}.chat-header{display:flex;align-items:center;gap:11px;min-height:58px;flex-shrink:0;padding:8px 18px;border-bottom:1px solid var(--border);background:var(--surface)}.chat-provider{display:grid;place-items:center;width:38px;height:38px;border-radius:var(--radius-lg);background:var(--client-soft);color:var(--client-ink)}.chat-heading{flex:1;min-width:0}.chat-heading strong{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:var(--text-base);font-weight:600}.chat-heading small{display:flex;align-items:center;gap:6px;margin-top:5px;font-size:var(--text-xs);color:var(--muted)}.chat-status-dot{width:6px;height:6px;background:var(--focus);border-radius:50%}.chat-status-dot.working{background:#c9a351}.chat-menu{position:relative}.chat-menu summary{list-style:none;display:grid;place-items:center;width:44px;height:44px;font-size:var(--text-2xl);cursor:pointer;border-radius:var(--radius-md)}.chat-menu summary::-webkit-details-marker{display:none}.chat-menu nav{position:absolute;right:0;top:44px;z-index:20;min-width:205px;background:var(--surface);border:1px solid var(--border);border-radius:var(--radius-lg);padding:5px;box-shadow:var(--shadow-lg),inset 0 1px 0 var(--card-highlight)}/* The same row as every other menu (ContextMenu, SessionCommonActions), at its type size. */
.chat-menu button{display:flex;align-items:center;gap:8px;width:100%;min-height:30px;border:0;background:none;text-align:left;padding:6px 9px;font-size:var(--text-sm);color:var(--ink-soft);border-radius:var(--radius-sm);cursor:pointer}.chat-menu button:hover,.chat-menu summary:hover{background:var(--fill);color:var(--ink)}.chat-timeline{min-height:0;flex:1;overflow-y:auto;overscroll-behavior:contain;padding:20px clamp(14px,5cqw,44px)}.chat-welcome{max-width:470px;text-align:center;margin:clamp(20px,6cqh,70px) auto 32px}.chat-welcome-mark{display:grid;place-items:center;width:64px;height:64px;border-radius:var(--radius-xl);background:var(--teal-soft);color:var(--teal);margin:0 auto 18px}.chat-welcome h3{font-size:var(--text-2xl);font-weight:550;letter-spacing:-.6px;margin:0 0 10px}.chat-welcome p{font-size:var(--text-md);line-height:1.8;color:var(--ink-soft);max-width:330px;margin:0 auto}.chat-context-chip{display:inline-block;max-width:100%;overflow-wrap:anywhere;margin-top:17px;border:1px solid var(--border);background:var(--bg);border-radius:var(--radius-xl);padding:7px 12px;font-size:var(--text-xs);color:var(--ink-soft)}.chat-message{max-width:48rem;margin:0 auto 12px;min-width:0}.chat-message.user{max-width:min(82%,48rem);margin-left:auto;margin-right:0;background:var(--sunken);border:1px solid var(--border);border-radius:var(--radius-xl) var(--radius-xl) var(--radius-sm) var(--radius-xl);padding:12px 16px;box-shadow:inset 0 1px 0 var(--card-highlight)}.chat-message-label{display:flex;align-items:center;gap:7px;color:var(--ink-soft);font-size:var(--text-xs);margin-bottom:6px;font-weight:600;letter-spacing:0}.chat-message.user .chat-message-label{color:var(--muted)}.chat-message :deep(.chat-markdown){font-family:var(--font-chat);font-size:var(--text-base);line-height:1.6;letter-spacing:0;overflow-wrap:anywhere}.chat-message :deep(p){margin:0 0 10px;white-space:pre-wrap}.chat-message :deep(p:last-child){margin-bottom:0}.chat-tool{max-width:48rem;margin:0 auto 6px;border:1px solid var(--border);border-radius:var(--radius-md);background:var(--sunken);overflow:hidden}.chat-tool summary{display:flex;align-items:center;gap:9px;min-height:46px;padding:8px 13px;cursor:pointer;list-style:none}.chat-tool summary::-webkit-details-marker{display:none}.chat-tool strong{font:500 var(--text-xs)/1.4 var(--mono);flex:1;min-width:0;overflow-wrap:anywhere}.chat-tool small{font-size:var(--text-xs);color:var(--muted)}.tool-indicator{color:var(--teal)}.tool-indicator.failed{color:var(--danger)}.tool-indicator.running{color:var(--warn-ink)}.chat-tool pre{white-space:pre-wrap;overflow-wrap:anywhere;font:var(--text-xs)/1.65 var(--mono);padding:13px;margin:0;max-height:360px;overflow:auto;border-top:1px solid var(--border)}.chat-tool p{padding:12px;font-size:var(--text-sm);color:var(--ink-soft)}.chat-primary,.chat-endpoint-confirm button{min-height:44px;padding:8px 15px;border-radius:var(--radius-md);border:1px solid var(--line);background:var(--surface);color:var(--ink-soft);font-size:var(--text-sm);cursor:pointer}.chat-primary,.chat-endpoint-confirm button:first-child{background:var(--teal);color:var(--on-accent);border-color:var(--teal)}.chat-boundary{text-align:center;margin:28px 0;border-block:1px solid var(--border);padding:17px;color:var(--ink-soft);font-size:var(--text-xs)}.chat-boundary>span{font-weight:600;color:var(--teal)}.chat-boundary p{margin:7px 0;overflow-wrap:anywhere}.chat-boundary small{font-size:var(--text-xs)}.chat-notice,.chat-alert,.chat-history-notice,.chat-delivery,.chat-native-note{padding:10px 16px;font-size:var(--text-sm);line-height:1.7;overflow-wrap:anywhere}.chat-notice{flex-shrink:0;background:var(--teal-soft);border-bottom:1px solid var(--teal-line);color:var(--teal-deep)}.chat-alert{background:var(--danger-soft);color:var(--danger-ink);border-radius:var(--radius-md);margin:8px 12px;flex-shrink:0}.chat-alert button{background:none;border:0;color:inherit;text-decoration:underline;min-height:44px;padding:8px}.chat-history-notice,.chat-delivery{text-align:center;color:var(--ink-soft);background:var(--bg);border-radius:var(--radius-md)}.chat-native-note{border:1px solid var(--border);border-radius:var(--radius-lg);text-align:center;margin:18px 0}.chat-link{min-height:44px;padding:8px;border:0;background:none;color:var(--teal);font-size:var(--text-sm);text-decoration:underline;cursor:pointer}/* The client's own mark turning is the whole indicator: it says which agent
   is thinking as well as that it is, which three neutral dots never did. */
.chat-working{display:flex;align-items:center;gap:8px;color:var(--ink-soft);font-size:var(--text-sm);margin:12px auto;max-width:48rem;animation:chat-working-in var(--duration-normal) var(--ease-out)}@keyframes chat-working-in{from{opacity:0;transform:translateY(4px)}}.chat-working-mark{animation:chat-spin 2.6s cubic-bezier(.45,.05,.35,1) infinite;transform-origin:50% 50%;will-change:transform}.chat-working-label{background:linear-gradient(90deg,var(--muted) 0%,var(--ink) 45%,var(--muted) 100%) 0 0/200% 100%;-webkit-background-clip:text;background-clip:text;color:transparent;animation:chat-text-shimmer 2.2s linear infinite}@keyframes chat-text-shimmer{to{background-position:-200% 0}}@keyframes chat-spin{0%{transform:rotate(0) scale(1);opacity:.75}24%{transform:rotate(155deg) scale(1.09);opacity:1}46%{transform:rotate(183deg) scale(1);opacity:.8}74%{transform:rotate(348deg) scale(1.06);opacity:1}100%{transform:rotate(360deg) scale(1);opacity:.75}}@keyframes chat-breathe{0%,100%{opacity:.6}50%{opacity:1}}.chat-pane.is-phone .chat-latest{align-self:flex-end;right:12px;min-height:36px;padding:6px 12px;font-size:var(--text-xs)}
.chat-latest{position:absolute;bottom:180px;align-self:center;border:1px solid var(--line);background:var(--surface);color:var(--teal);box-shadow:var(--shadow-md);border-radius:30px;min-height:44px;padding:8px 14px;font-size:var(--text-sm)}.chat-composer{flex-shrink:0;width:calc(100% - 32px);max-width:48rem;margin:0 auto 12px;padding:12px 13px 7px;background:var(--glass-surface);backdrop-filter:var(--glass-blur);-webkit-backdrop-filter:var(--glass-blur);border:1px solid var(--line);border-radius:var(--radius-xl);box-shadow:var(--shadow-lg),inset 0 1px 0 var(--card-highlight);padding-bottom:max(7px,env(safe-area-inset-bottom));transition:border-color var(--duration-fast) var(--ease-out),box-shadow var(--duration-fast) var(--ease-out)}.chat-composer>textarea{display:block;width:100%;resize:vertical;min-height:66px;max-height:200px;border:0;background:none;box-shadow:none;outline:0;font-family:inherit;font-size:var(--text-base);line-height:1.7;color:var(--ink);padding:3px 3px 10px}.chat-composer>textarea::placeholder{color:var(--muted)}.chat-composer-controls{display:flex;align-items:center;gap:8px;min-width:0}/* The chips themselves are ChipMenu; what stays here is only what goes
   inside their panels. */
.chat-endpoint{max-width:min(72%,320px)}.chat-endpoint>summary>.balance-badge{flex-shrink:0}.chat-endpoint:has(.balance-badge) :deep(.chip-menu-label){min-width:3em}
.chat-model{max-width:min(34%,170px)}
/* Wide enough for the line it holds, not for the chip it hangs from: these
   panels shrink-to-fit inside a chip barely a hundred pixels across, which is
   how a description ended up wrapping three times with room to spare. */
.chat-endpoint :deep(.chip-menu-panel),.chat-model :deep(.chip-menu-panel),.chat-permission :deep(.chip-menu-panel){min-width:190px;width:max-content;max-width:min(292px,78cqw);max-height:min(340px,66vh);overflow-y:auto;padding:5px}
.chat-model nav button strong{display:block;font-size:var(--text-sm);font-weight:600}
.chat-model nav button small{display:block;margin-top:2px;font-size:var(--text-xs);color:var(--muted);white-space:normal;line-height:1.5}
.chat-model nav button{white-space:normal}
.chat-endpoint nav button,.chat-model nav button{display:block;width:100%;min-height:38px;padding:8px 10px;border:0;border-radius:var(--radius-md);background:none;text-align:left;font-size:var(--text-sm);color:var(--ink-soft);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;cursor:pointer}
.chat-endpoint nav button:hover:not(:disabled),.chat-model nav button:hover:not(:disabled){background:var(--fill)}
.chat-endpoint nav button.selected,.chat-model nav button.selected{color:var(--accent-ink);font-weight:600;background:var(--selected)}
.chat-effort{max-width:132px}
/* The unattended mode is the one that stops asking, so it is the one that has
   to look different from every other chip rather than merely selected. */
.chat-permission{max-width:150px}
/* A panel is as wide as its longest line, within reason. This one was left out
   of the rules above and fell back to the browser's own buttons, in a column so
   narrow that a sentence wrapped every three characters. */
/* Tall enough that a fixed list of four does not scroll by two pixels, and
   still bounded for a short screen. */
.chat-permission :deep(.chip-menu-panel){min-width:214px}
.chat-permission-body>header{padding:8px 10px 6px;font-size:var(--text-xs);color:var(--muted)}
.chat-permission nav button{display:block;width:100%;padding:8px 10px;border:0;border-radius:var(--radius-md);background:none;text-align:left;white-space:normal;cursor:pointer}
.chat-permission nav button+button{margin-top:1px}
.chat-permission nav button>span{display:flex;align-items:center;justify-content:space-between;gap:8px}
.chat-permission nav button strong{font-size:var(--text-sm);font-weight:600;color:var(--ink-soft)}
.chat-permission nav button small{display:block;margin-top:3px;font-size:var(--text-xs);line-height:1.6;color:var(--muted)}
.chat-permission nav button:hover{background:var(--fill)}
.chat-permission nav button.selected{background:var(--selected)}
.chat-permission nav button.selected strong{color:var(--teal)}
.chat-permission nav button.danger strong{color:#a85c4e}
.chat-permission nav button.danger:hover,.chat-permission nav button.danger.selected{background:var(--danger-soft)}
.chat-permission nav button.danger small{color:#b07a70}
.chat-effort :deep(.chip-menu-panel){padding:13px 14px 10px}
.chat-effort-note{font-size:var(--text-xs);color:var(--muted);line-height:1.5;margin:0 0 10px}
.chat-effort-body{width:218px;max-width:74cqw}

.chat-effort-body header{display:flex;align-items:baseline;justify-content:space-between;gap:8px;margin-bottom:11px}
.chat-effort-body header strong{font-size:var(--text-sm);font-weight:600;color:var(--ink)}
.chat-effort-body header em{font-style:normal;font-size:var(--text-sm);font-weight:600;color:var(--teal)}
.chat-effort.vivid .chat-effort-body header em{background:linear-gradient(92deg,var(--accent-ink),#4f7fd8);-webkit-background-clip:text;background-clip:text;color:transparent}
.chat-effort-body footer{display:flex;justify-content:space-between;margin-top:7px;font-size:var(--text-xs);color:var(--muted)}
/* A real range input stays on top, invisible: dragging, clicking the track and
   arrow keys keep working while the pill below is free to be styled. */
.chat-effort-slider{position:relative;height:26px;display:flex;align-items:center}
.chat-effort-track{position:absolute;inset:0;border-radius:var(--radius-lg);background:var(--fill-hover)}
.chat-effort-track i{position:absolute;top:50%;width:4px;height:4px;margin:-2px 0 0 -2px;border-radius:50%;background:var(--line-strong)}
.chat-effort-track i:first-child{margin-left:5px}
.chat-effort-track i:last-child{margin-left:-9px}
.chat-effort-fill{position:absolute;left:0;top:0;bottom:0;border-radius:var(--radius-lg);width:calc(11px + (100% - 22px)*var(--effort-fill));min-width:26px;background:var(--teal);transition:width .16s ease,background .16s ease}
.chat-effort-thumb{position:absolute;top:50%;left:calc(11px + (100% - 22px)*var(--effort-fill));width:22px;height:22px;margin:-11px 0 0 -11px;border-radius:50%;background:var(--surface);box-shadow:var(--shadow-md);transition:left .16s ease}
.chat-effort-slider input{position:absolute;inset:0;width:100%;height:100%;margin:0;opacity:0;cursor:grab;-webkit-appearance:none;appearance:none}
.chat-effort-slider input:active{cursor:grabbing}
.chat-effort-slider input:disabled{cursor:not-allowed}
.chat-effort-slider input:disabled~.chat-effort-fill,.chat-effort-slider input:disabled~.chat-effort-thumb{opacity:.45}
.chat-effort-slider input:focus-visible~.chat-effort-thumb{outline:2px solid var(--focus);outline-offset:2px}
/* The top levels cost real time and money, so they read as charged rather than
   as one more step: a gradient, a lift, and a slow pulse. */
.chat-effort.vivid .chat-effort-fill{background:linear-gradient(92deg,var(--accent-ink),#3f9fd0 62%,#6b6ee0);box-shadow:0 0 16px #4f8fd855}
.chat-effort.vivid .chat-effort-thumb{box-shadow:0 1px 6px #243b4c33,0 0 0 3px #6b6ee02e;animation:effort-pulse 2.1s ease-in-out infinite}
@keyframes effort-pulse{0%,100%{box-shadow:0 1px 6px #243b4c33,0 0 0 3px #6b6ee02e}50%{box-shadow:0 1px 6px #243b4c33,0 0 0 7px #6b6ee014}}.chat-send{margin-left:auto;flex-shrink:0;width:40px;height:40px;border:0;border-radius:var(--radius-full);background:var(--accent);color:var(--on-accent);font-size:var(--text-2xl);cursor:pointer;box-shadow:0 1px 3px var(--accent-glow);transition:background var(--duration-fast),box-shadow var(--duration-fast),transform var(--duration-fast) var(--ease-out)}.chat-send:hover:not(:disabled){background:var(--accent-hover);box-shadow:var(--shadow-glow)}.chat-send:active:not(:disabled){transform:scale(.95)}.chat-send:disabled{opacity:.45;cursor:not-allowed}.chat-stop{background:var(--danger);font-size:var(--text-xl)}.chat-composer footer{display:flex;justify-content:space-between;gap:6px;flex-wrap:wrap;font-size:var(--text-xs);color:var(--muted);margin:7px 4px 0}.chat-endpoint-confirm{padding:12px;border:1px solid var(--warn-line);background:#fffaee;border-radius:var(--radius-md);margin-bottom:10px;font-size:var(--text-sm);color:#86713e}.chat-endpoint-confirm p{line-height:1.7;font-size:var(--text-xs)}.chat-endpoint-confirm>div{display:flex;flex-wrap:wrap;gap:8px}.chat-pane button:disabled{cursor:not-allowed;opacity:.5}.chat-pane button:focus-visible,.chat-pane summary:focus-visible,.chat-pane select:focus-visible,.chat-pane input:focus-visible,.chat-pane textarea:focus-visible{outline:2px solid var(--focus);outline-offset:3px}.sr-only{position:absolute;width:1px;height:1px;padding:0;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}@container(max-width:480px){.chat-header{padding:8px 12px}.chat-timeline{padding:17px 13px}.chat-message.user{max-width:92%}.chat-composer{width:calc(100% - 16px);margin:0 auto 8px;padding:10px 10px max(7px,env(safe-area-inset-bottom));border-radius:var(--radius-lg)}.chat-endpoint{max-width:52%}.chat-composer footer{font-size:var(--text-xs)}.chat-message :deep(.chat-markdown){font-size:var(--text-base)}.chat-tool small{display:none}.chat-welcome h3{font-size:var(--text-2xl)}.chat-menu nav{min-width:190px}}@media(prefers-reduced-motion:reduce){.chat-pane *{scroll-behavior:auto}.chat-working-mark,.chat-working-label{animation:none}.chat-effort-fill,.chat-effort-thumb{transition:none}.chat-effort.vivid .chat-effort-thumb{animation:none}}
.chat-welcome .chat-trust-note{font-size:var(--text-xs);color:var(--ink-soft);margin-top:16px;line-height:1.8}.chat-composer footer,.chat-tool small,.chat-history-notice,.chat-heading small{color:var(--ink-soft)}
.chat-usage{display:flex;align-items:center;gap:8px;min-width:0}
.chat-commands{display:flex;flex-direction:column;gap:1px;margin-bottom:8px;padding:5px;border:1px solid var(--border);border-radius:var(--radius-md);background:var(--surface);box-shadow:var(--shadow-lg);max-height:216px;overflow-y:auto}
.chat-commands button{display:block;width:100%;text-align:left;border:0;border-radius:var(--radius-sm);background:none;padding:7px 9px;font:var(--text-sm) var(--mono);color:var(--ink-soft);cursor:pointer}
.chat-commands button.highlighted{background:var(--selected);color:var(--ink)}
.chat-commands small{padding:6px 9px 2px;font-size:var(--text-xs);color:var(--muted)}
.chat-attach{flex-shrink:0;width:40px;height:40px;display:grid;place-items:center;border:1px solid var(--line);border-radius:var(--radius-full);background:var(--surface);color:var(--ink-soft);cursor:pointer;transition:background var(--duration-fast),color var(--duration-fast),border-color var(--duration-fast)}
.chat-attach:hover:not(:disabled){background:var(--fill);color:var(--ink);border-color:var(--line-strong)}
.chat-attachments{list-style:none;display:flex;flex-wrap:wrap;gap:7px;margin:0 0 9px;padding:0}
.chat-attachments li{display:flex;align-items:center;gap:6px;max-width:100%;min-width:0;border:1px solid var(--border);border-radius:var(--radius-md);background:var(--sunken);padding:5px 5px 5px 9px;font-size:var(--text-xs);color:var(--ink-soft)}
.chat-attachment-name{min-width:0;max-width:190px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.chat-attachments small{color:var(--muted);font-size:var(--text-xs);white-space:nowrap}
.chat-attachments button{display:grid;place-items:center;width:30px;height:30px;flex-shrink:0;border:0;border-radius:var(--radius-sm);background:none;color:var(--muted);cursor:pointer}
.chat-attachments button:hover{background:var(--fill);color:var(--ink-soft)}
/* The visible control stays chip-sized while its touch area reaches 44px, so a
   thumb on a phone does not have to hit a 30px dot. */
.chat-attachments button{position:relative}
.chat-attachments button::after{content:'';position:absolute;inset:-7px}
@container(max-width:480px){.chat-attachment-name{max-width:140px}}
/* Subagent progress is a different kind of content from the tool's own input
   and result, so it reads as a distinct block rather than more of the same. */
.tool-activity{background:var(--violet-soft);color:var(--info-ink);border-bottom:1px solid var(--border)}
.chat-tool small{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:45%}
/* A mention row shows the file and where it is, since names repeat across a repo. */
.chat-mentions button{display:flex;align-items:baseline;gap:8px}
.chat-mentions strong{font-weight:550;flex-shrink:0}
.chat-mentions small{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;direction:rtl;text-align:right;color:var(--muted)}
/* A subagent's own calls, indented by how deep they ran. */
.chat-tool-children{border-top:1px solid var(--border)}
.chat-tool-nested+.chat-tool-nested{border-top:1px solid var(--border)}
.chat-tool-nested>summary{display:flex;align-items:center;gap:8px;min-height:32px;padding:5px 13px 5px calc(13px + var(--depth,1) * 16px);cursor:pointer;list-style:none}
.chat-tool-nested>summary::-webkit-details-marker{display:none}
.chat-tool-nested>summary strong{font:var(--text-xs) var(--mono);flex:1;min-width:0;overflow-wrap:anywhere}
.chat-tool-nested pre{padding-left:calc(13px + var(--depth,1) * 16px)}
.chat-message-foot{display:flex;align-items:center;gap:6px;min-height:18px;margin-top:2px}
.chat-pane.is-phone .chat-message.user .chat-message-foot{display:none}
.chat-message.user .chat-message-foot{display:block}
.chat-turn-usage{margin:0;font:var(--text-xs) var(--mono);color:var(--faint);font-variant-numeric:tabular-nums;cursor:default}
.chat-tool-stats{font:var(--text-xs) var(--mono);color:var(--muted);white-space:nowrap}
.chat-tool-stalled{font-style:normal;font-size:var(--text-xs);color:var(--warn-ink);background:var(--warn-soft);border:1px solid var(--warn-line);border-radius:var(--radius-sm);padding:1px 6px;white-space:nowrap}
.subagent-dock{flex-shrink:0;display:flex;flex-direction:column;max-height:min(46vh,420px);width:calc(100% - 32px);max-width:48rem;margin:0 auto 8px;background:var(--surface);border:1px solid var(--border);border-radius:var(--radius-lg);box-shadow:var(--shadow-md),inset 0 1px 0 var(--card-highlight);overflow:hidden}
.subagent-dock>header{display:flex;align-items:center;gap:6px;padding-right:8px}
.subagent-dock-head{flex:1;min-width:0;display:flex;align-items:center;gap:8px;min-height:36px;padding:6px 12px;border:0;background:none;font:inherit;font-size:var(--text-xs);color:var(--ink-soft);cursor:pointer;text-align:left}
.subagent-dock-head strong{font-weight:600;color:var(--ink);white-space:nowrap}
.subagent-dock-head small{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--muted)}
.subagent-dock-chevron{margin-left:auto;flex-shrink:0;transform:rotate(270deg);color:var(--muted)}.subagent-dock.collapsed .subagent-dock-chevron{transform:rotate(90deg)}
.subagent-dock-dot{flex-shrink:0;width:7px;height:7px;border-radius:50%;background:var(--teal);animation:chat-breathe 2.6s ease-in-out infinite}.subagent-dock-dot.idle{background:var(--muted);animation:none}
.subagent-dock-clear{flex-shrink:0;border:0;background:none;padding:4px 6px;font:inherit;font-size:var(--text-xs);color:var(--teal);cursor:pointer}
.subagent-dock>ul{list-style:none;margin:0;padding:0;overflow-y:auto;overscroll-behavior:contain;border-top:1px solid var(--border)}
.subagent-item+.subagent-item{border-top:1px solid var(--border)}
.subagent-row{display:flex;align-items:center}
.subagent-toggle{flex:1;min-width:0;display:flex;align-items:center;gap:8px;min-height:34px;padding:5px 12px;border:0;background:none;font:inherit;font-size:var(--text-xs);color:var(--ink-soft);cursor:pointer;text-align:left}
.subagent-toggle:hover{background:var(--fill)}
.subagent-toggle strong{flex:0 1 auto;max-width:40%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-weight:600;color:var(--ink)}
.subagent-toggle small{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--muted);font-size:var(--text-xs)}
.subagent-chevron{flex-shrink:0;transform:rotate(90deg);color:var(--muted)}.subagent-item.open .subagent-chevron{transform:rotate(270deg)}
.subagent-dismiss{flex-shrink:0;width:28px;height:28px;margin-right:6px;border:0;border-radius:var(--radius-sm);background:none;color:var(--muted);font-size:var(--text-lg);cursor:pointer}.subagent-dismiss:hover{background:var(--fill);color:var(--ink)}
.subagent-item.completed .subagent-toggle strong,.subagent-item.failed .subagent-toggle strong{color:var(--ink-soft)}
.subagent-preview{border-top:1px solid var(--border);background:var(--sunken)}
.subagent-preview pre{margin:0;padding:10px 13px;white-space:pre-wrap;overflow-wrap:anywhere;font:var(--text-xs)/1.7 var(--mono);max-height:220px;overflow:auto}
.subagent-preview .chat-tool-nested pre{max-height:180px;border-top:1px solid var(--border)}
.subagent-preview .chat-tool-nested small{flex-shrink:1;min-width:0;max-width:45%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:var(--text-xs);color:var(--muted)}
.subagent-kind{vertical-align:-1px;margin-right:5px;color:var(--muted)}
.subagent-empty{margin:0;padding:10px 13px;font-size:var(--text-xs);color:var(--muted)}
.chat-pane:not(.is-phone) .subagent-dock{margin-inline:max(16px,calc((100% - 860px) / 2))}
.chat-pane.is-phone .subagent-dock{margin:0 8px 6px}
@container(max-width:480px){.subagent-dock{margin:0 8px 6px}.subagent-toggle .chat-tool-stats{display:none}.subagent-toggle strong{max-width:55%}}
@container(max-width:480px){.chat-tool-stats{display:none}}
@media(prefers-reduced-motion:reduce){.subagent-dock-dot{animation:none}}
.chat-tool-chevron{transform:rotate(90deg)}.chat-tool[open] .chat-tool-chevron{transform:rotate(270deg)}
.chat-end-confirm{flex-shrink:0;background:var(--danger-soft);color:var(--danger-ink);border-bottom:1px solid var(--danger-line);padding:11px 16px;font-size:var(--text-sm);line-height:1.7}.chat-end-confirm p{margin:0 0 8px}.chat-end-confirm>div{display:flex;gap:8px;flex-wrap:wrap}.chat-end-confirm button{min-height:44px;padding:8px 13px;border:1px solid var(--danger-line);border-radius:var(--radius-md);background:var(--surface);color:var(--danger)}.chat-end-confirm button:first-child{background:var(--danger);color:var(--on-accent);border-color:var(--danger)}.chat-menu .chat-end-button{color:var(--danger)}

/* Compact density and a single focus ring for the composer. The outline is
 * intentionally on the whole card, so keyboard focus is visible without a
 * distracting rectangle around the textarea alone. */
.chat-message :deep(.chat-markdown){font-size:var(--text-md);line-height:1.7}

.chat-composer:focus-within{border-color:var(--focus);box-shadow:0 0 0 3px var(--accent-soft),var(--shadow-lg),inset 0 1px 0 var(--card-highlight)}
.chat-composer>textarea{font-size:var(--text-md);line-height:1.6}
.chat-composer>textarea:focus-visible{outline:0;box-shadow:none}

.chat-header{display:none}
.chat-floating-menu{position:absolute;top:6px;right:9px;z-index:20}
.chat-floating-menu>summary{list-style:none;display:grid;place-items:center;width:40px;height:40px;border-radius:var(--radius-md);color:var(--ink-soft);font-size:var(--text-2xl);cursor:pointer}
.chat-floating-menu>summary::-webkit-details-marker{display:none}
.chat-floating-menu>summary:hover,.chat-floating-menu[open]>summary{background:var(--fill);color:var(--ink)}
.chat-floating-menu nav{top:40px}
.chat-menu-status{display:flex;align-items:center;gap:6px;padding:8px 9px;border-bottom:1px solid var(--border);font-size:var(--text-xs);color:var(--ink-soft)}
.chat-timeline{padding-top:52px}
.chat-pane.has-tab-menu .chat-timeline{padding-top:24px}
@container(max-width:480px){.chat-floating-menu{top:4px;right:5px}.chat-floating-menu>summary{width:44px;height:44px}.chat-timeline{padding-top:49px}}
.chat-pane.has-tab-menu .chat-timeline{padding-top:24px}
.chat-tab-menu{position:fixed;top:auto;right:auto;z-index:60;display:inline-flex}
.chat-tab-menu>summary{width:28px;height:28px;font-size:var(--text-xl);border-radius:var(--radius-sm)}
.chat-tab-menu nav{top:32px;right:0}
.chat-menu button>svg{flex:none}
.chat-menu-divider{border:0;border-top:1px solid var(--line);margin:4px 6px}.chat-menu summary:focus-visible,.chat-menu button:focus-visible{outline:2px solid var(--focus);outline-offset:2px}
@media(max-width:520px){.chat-tab-menu>summary{width:32px;height:32px;font-size:var(--text-xl)}.chat-tab-menu nav{top:36px}}
/* Teleported to the body, outside this pane and the shell, so it carries its own phone class. */
.chat-tab-menu.is-phone>summary{width:44px;height:44px}

/* The composer is deliberately denser than the message timeline.  Keeping
 * the desktop controls compact makes the input feel like a tool bar instead
 * of a second card, while the mobile rule below restores comfortable touch
 * targets. */
.chat-composer{width:calc(100% - 24px);margin:0 auto 8px;padding:8px 9px max(5px,env(safe-area-inset-bottom));border-radius:var(--radius-md)}
.chat-composer>textarea{min-height:48px;max-height:160px;padding:1px 2px 5px}
.chat-composer-controls{gap:5px}
.chat-composer .chat-attach{width:30px;height:30px;min-width:30px;min-height:0;border-radius:var(--radius-md)}
.chat-send{width:30px;height:30px;min-width:30px;min-height:0;border-radius:var(--radius-md);font-size:var(--text-xl)}
.chat-stop{font-size:var(--text-md)}
.chat-composer footer{margin:3px 2px 0;font-size:var(--text-xs);line-height:1.25}

/* Width controls layout; it does not decide how the controls are pointed at. A
   narrow pane on a desktop is still a mouse, so only spacing changes here. */
@container(max-width:480px){
  .chat-composer{width:calc(100% - 16px);margin:0 auto 8px;padding:10px 10px max(7px,env(safe-area-inset-bottom));border-radius:var(--radius-lg)}
  .chat-composer>textarea{min-height:66px;max-height:200px;padding:3px 3px 10px}
  
  .chat-endpoint{max-width:52%}
  .chat-composer footer{margin-top:5px;font-size:var(--text-xs);line-height:1.4}
}

/* Touch targets follow the pointing device, so a phone or tablet gets 44px at
   any pane width and a mouse keeps the compact tool bar. The 16px input font
   is here too: it stops iOS zooming the page on focus. */
@media(pointer:coarse){
  .chat-menu button{min-height:44px}
  .chat-composer>textarea{font-size:var(--input-text)}
  /* A finger needs a taller pill to drag along. */
  .chat-effort-body{width:236px}
  .chat-effort-slider{height:34px}
  .chat-effort-track,.chat-effort-fill{border-radius:var(--radius-xl)}
  .chat-effort-fill{width:calc(14px + (100% - 28px)*var(--effort-fill));min-width:34px}
  .chat-effort-thumb{left:calc(14px + (100% - 28px)*var(--effort-fill));width:28px;height:28px;margin:-14px 0 0 -14px}
  .chat-composer .chat-attach{width:44px;height:44px;min-width:44px;min-height:44px;border-radius:var(--radius-lg)}
  .chat-composer .chat-send{width:44px;height:44px;min-width:44px;min-height:44px;border-radius:var(--radius-lg);font-size:var(--text-2xl)}
  .chat-attachments button{width:44px;height:44px}
}

/* Startup: the client announces its models, permissions and commands in its
   handshake, so until then the composer says it is starting and the chips
   arrive together instead of popping in one by one. */
.chat-boot{display:flex;flex-direction:column;align-items:center;gap:10px;margin-top:18px;font-size:var(--text-xs);color:var(--ink-soft)}
.chat-boot-bar{position:relative;display:block;width:148px;height:3px;border-radius:var(--radius-xs);background:var(--teal-soft);overflow:hidden}
.chat-boot-bar i{position:absolute;inset:0 auto 0 0;width:40%;border-radius:var(--radius-xs);background:var(--teal);animation:chat-boot-slide 1.25s cubic-bezier(.45,.05,.35,1) infinite}
@keyframes chat-boot-slide{0%{transform:translateX(-100%)}100%{transform:translateX(250%)}}
.chat-boot-chip{display:inline-flex;align-items:center;gap:6px;flex-shrink:0;height:30px;padding:0 11px;border-radius:var(--radius-md);font-size:var(--text-xs);color:var(--ink-soft);background:linear-gradient(90deg,var(--teal-soft) 0%,var(--surface) 50%,var(--teal-soft) 100%) 0 0/200% 100%;border:1px dashed var(--line);animation:chat-shimmer 1.6s linear infinite}
@keyframes chat-shimmer{to{background-position:-200% 0}}
/* The placeholder chip has no exit on purpose: it holds the place the real
   chips arrive in, and fading out beside them -- Vue waits out its endless
   shimmer, not the fade -- crowded the row and squeezed every chip for a
   moment in a narrow pane. The real chips bring their own entrance. */
.chat-boot-chip{animation:chat-shimmer 1.6s linear infinite,chat-chip-in .24s ease-out both}
.chat-boot-enter-active,.chat-boot-leave-active{transition:opacity .24s ease,transform .24s ease}
.chat-boot-enter-from,.chat-boot-leave-to{opacity:0;transform:translateY(4px)}
.chat-model,.chat-permission,.chat-effort{animation:chat-chip-in .34s cubic-bezier(.2,.8,.2,1) backwards}
.chat-permission{animation-delay:.06s}
@keyframes chat-chip-in{from{opacity:0;transform:translateY(5px) scale(.96)}to{opacity:1;transform:none}}
.chat-model-custom{display:flex;gap:6px;margin:5px 3px 3px;padding-top:6px;border-top:1px solid var(--line);position:sticky;bottom:-5px;background:var(--surface);padding-bottom:5px}
/* The ID a model goes out as, under the name it is known by. */
.chat-model nav button .chat-model-id{display:block;margin-top:2px;font-family:var(--mono);font-size:var(--text-xs);color:var(--muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.chat-model-custom input{flex:1;min-width:0;height:30px;padding:0 8px;border:1px solid var(--line);border-radius:var(--radius-sm);font:inherit;font-size:var(--text-xs);background:var(--surface);color:inherit}
.chat-model-custom button{height:30px;padding:0 10px;border:0;border-radius:var(--radius-sm);background:var(--teal);color:var(--on-accent);font-size:var(--text-xs);cursor:pointer}
.chat-model-hint{margin:4px 5px 3px;font-size:var(--text-xs);line-height:1.5;color:var(--muted);white-space:normal}
@media(prefers-reduced-motion:reduce){.chat-boot-bar i,.chat-boot-chip,.chat-model,.chat-permission,.chat-effort{animation:none}}

/* The empty conversation is the screen seen most often, and it was a white
   page with a sentence in it. A little light behind it, and four ways in. */
.chat-timeline.is-empty{position:relative;isolation:isolate;overflow-x:hidden}
/* The virtualizer keeps the rows in view where they were as others are measured
   or put in front; the browser's own anchoring would correct the same shift twice. */
/* Rows come and go as the conversation scrolls. Contained, laying them out
   stays inside the conversation instead of the whole page relaying out each
   frame (149ms → 18ms at the 95th percentile on a slowed CPU). */
.chat-timeline{overflow-anchor:none;contain:strict}
/* Clicked, the conversation takes focus, so ⌘/Ctrl+F and the arrow keys reach it; it shows no ring for that. */
.chat-timeline:focus{outline:none}
.chat-timeline.is-empty::before{content:"";position:absolute;inset:0;z-index:-1;pointer-events:none;background-image:radial-gradient(circle,var(--line-strong) 1px,transparent 1.3px);background-size:22px 22px;background-position:center 8px;-webkit-mask-image:radial-gradient(ellipse 70% 60% at 50% 32%,#000 10%,transparent 75%);mask-image:radial-gradient(ellipse 70% 60% at 50% 32%,#000 10%,transparent 75%)}
.chat-welcome{position:relative;isolation:isolate;max-width:560px}
.chat-welcome::before{content:"";position:absolute;z-index:-1;left:50%;top:-40px;width:min(620px,100cqw);height:340px;transform:translateX(-50%);background:radial-gradient(closest-side at 38% 42%,color-mix(in srgb, var(--accent) 14%, transparent),transparent),radial-gradient(closest-side at 64% 52%,color-mix(in srgb, var(--violet) 12%, transparent),transparent);pointer-events:none}
.chat-welcome-mark{position:relative;background:linear-gradient(140deg,var(--surface) 0%,var(--accent-soft) 55%,var(--violet-soft) 100%);box-shadow:0 10px 30px #0c837622,inset 0 0 0 1px #ffffffb0;animation:chat-float 6s ease-in-out infinite}
.chat-welcome-mark::after{content:"";position:absolute;inset:-7px;border-radius:28px;border:1px dashed color-mix(in srgb, var(--accent) 20%, transparent)}
@keyframes chat-float{0%,100%{transform:translateY(0)}50%{transform:translateY(-4px)}}
.chat-welcome h3{background:linear-gradient(100deg,var(--ink) 30%,var(--teal) 75%,var(--violet));-webkit-background-clip:text;background-clip:text;color:transparent}
.chat-context-chip{display:inline-flex;align-items:center;gap:6px;background:color-mix(in srgb, var(--surface) 85%, transparent);backdrop-filter:blur(4px);cursor:help}
.chat-starters{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px;margin:24px auto 0;text-align:left}
.chat-starters button{display:flex;align-items:flex-start;gap:11px;padding:12px 13px;border:1px solid var(--border);border-radius:var(--radius-lg);background:color-mix(in srgb, var(--surface) 90%, transparent);backdrop-filter:blur(4px);color:var(--ink);cursor:pointer;font:inherit;text-align:left;box-shadow:var(--shadow-sm);transition:transform .18s ease,box-shadow .18s ease,border-color .18s ease;animation:chat-starter-in .42s cubic-bezier(.2,.8,.2,1) both;animation-delay:var(--starter-delay)}
.chat-starters button:hover{transform:translateY(-2px);border-color:var(--teal-line);box-shadow:var(--shadow-lg)}
.chat-starters strong{display:block;font-size:var(--text-md);font-weight:600;margin-bottom:3px}
.chat-starters small{display:block;font-size:var(--text-xs);line-height:1.5;color:var(--ink-soft)}
.chat-starter-icon{display:grid;place-items:center;flex-shrink:0;width:30px;height:30px;border-radius:var(--radius-md);background:var(--teal-soft);color:var(--teal);--icon-color:var(--teal)}
.chat-starter-icon[data-tone="git"]{background:var(--warn-soft);--icon-color:#c26a2d}
.chat-starter-icon[data-tone="search"]{background:var(--violet-soft);--icon-color:var(--violet)}
.chat-starter-icon[data-tone="check"]{background:var(--info-soft);--icon-color:#3b74b8}
@keyframes chat-starter-in{from{opacity:0;transform:translateY(6px)}to{opacity:1;transform:none}}
.chat-endpoint nav button{white-space:normal}
.chat-endpoint nav button strong{display:flex;align-items:baseline;gap:12px;font-size:var(--text-sm);font-weight:600;white-space:nowrap}.chat-endpoint nav button strong>span{min-width:0;overflow:hidden;text-overflow:ellipsis}.chat-endpoint nav button strong>.balance-badge{margin-left:auto}
.chat-endpoint nav button small{display:block;margin-top:2px;font-size:var(--text-xs);line-height:1.5;color:var(--muted);font-weight:400;overflow-wrap:anywhere}
@container(max-width:480px){.chat-starters{grid-template-columns:1fr}}
@media(prefers-reduced-motion:reduce){.chat-welcome-mark,.chat-starters button{animation:none}}

.chat-menu-section{margin:6px 8px 3px;padding-top:8px;border-top:1px solid var(--line);font-size:var(--text-xs);font-weight:600;letter-spacing:.3px;color:var(--muted)}
.chat-menu-section small{display:block;margin-top:2px;font-weight:400;letter-spacing:0;line-height:1.45}
.chat-menu-note{margin:6px 10px 6px;font-size:var(--text-xs);line-height:1.45;color:var(--warn-ink)}
.chat-endpoint nav button:disabled{opacity:.5;cursor:not-allowed}
.chat-endpoint nav button.chat-endpoint-manage{display:flex;align-items:center;gap:7px;min-height:32px;margin-top:4px;border-top:1px solid var(--line);border-radius:0 0 var(--radius-md) var(--radius-md);font-size:var(--text-xs);color:var(--ink-soft)}
.chat-handoff-choice strong{display:flex!important;align-items:center;gap:6px}
.chat-endpoint-target{margin-left:8px;font-weight:500}.chat-endpoint-target>.balance-badge{margin-left:6px}
.chat-handoff-confirm{border-color:var(--faint);background:var(--violet-soft);color:var(--violet)}
.chat-handoff-confirm>div button:first-child{background:var(--violet);color:var(--on-accent);border-color:var(--violet)}

/* A run of tool calls: one card whose body is the calls themselves. */
.chat-tool-solo{display:contents}
/* One row of the conversation: its own margins stay inside it, so its measured
   height is the room it takes. Only the rows near the view are in the page (the
   virtualizer), so none needs skipping, and each is measured at its real size. */
.chat-history-older{min-height:28px;text-align:center;color:var(--muted);font-size:var(--text-xs)}
.chat-search{position:absolute;top:8px;right:12px;z-index:25;display:flex;align-items:center;gap:2px;width:min(400px,calc(100% - 24px));height:38px;padding:0 5px 0 12px;border:1px solid var(--border);border-radius:var(--radius-full);background:var(--surface);box-shadow:var(--shadow-lg);transition:border-color .15s}
.chat-search:focus-within{border-color:var(--teal-line)}
/* The floating session menu sits in the corner when the pane has no tab of its own. */
.chat-pane:not(.has-tab-menu) .chat-search{right:52px;width:min(400px,calc(100% - 64px))}
.chat-search-icon{color:var(--muted);flex-shrink:0;margin-right:4px}
.chat-search .chat-search-input{flex:1;min-width:0;height:100%;border:0;border-radius:0;background:none;box-shadow:none;font:inherit;font-size:var(--text-sm);color:var(--ink);padding:0;outline:none;appearance:none}
.chat-search .chat-search-input:focus{border:0;box-shadow:none;outline:none}
.chat-search-input::placeholder{color:var(--muted)}
.chat-search-count{font-size:var(--text-xs);font-variant-numeric:tabular-nums;color:var(--muted);white-space:nowrap;padding:0 6px}
.chat-search-divider{width:1px;height:18px;background:var(--line);margin:0 3px;flex-shrink:0}
.chat-search-button{display:inline-grid;place-items:center;width:28px;height:28px;flex-shrink:0;border:0;border-radius:var(--radius-full);background:none;color:var(--ink-soft);cursor:pointer}
.chat-search-button:hover:not(:disabled){background:var(--sunken);color:var(--ink)}
.chat-search-button:disabled{opacity:.35;cursor:default}
.chat-search-up{transform:rotate(-90deg)}.chat-search-down{transform:rotate(90deg)}
.chat-search-notice{position:absolute;top:calc(100% + 6px);right:8px;margin:0;padding:5px 10px;border-radius:var(--radius-md);background:var(--surface);box-shadow:var(--shadow-md);font-size:var(--text-xs);color:var(--muted);white-space:nowrap}
::highlight(chat-search){background-color:#f5c54266;color:inherit}
::highlight(chat-search-current){background-color:#f59e0bcc;color:#1a1a1a}
.chat-tool-run{max-width:48rem;margin:0 auto 6px;border:1px solid var(--border);border-radius:var(--radius-md);background:var(--sunken);overflow:hidden}
.chat-tool-run>summary{display:flex;align-items:center;gap:9px;min-height:46px;padding:8px 13px;cursor:pointer;list-style:none}
.chat-tool-run>summary::-webkit-details-marker{display:none}
.chat-tool-run>summary strong{font-size:var(--text-sm);font-weight:600;flex:1;min-width:0}
.chat-tool-run>summary small{font:var(--text-xs) var(--mono);color:var(--ink-soft);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:55%}
.chat-tool-run[open]>summary{border-bottom:1px solid var(--border)}
.chat-tool-run[open]>summary .chat-tool-chevron{transform:rotate(270deg)}
.chat-tool-run>.chat-tool{margin:0;border:0;border-radius:0;background:none}
.chat-tool-run>.chat-tool+.chat-tool{border-top:1px solid var(--border)}
.chat-tool-run>.chat-tool summary{min-height:38px;padding-left:22px}

/* Messages waiting for the turn to end. */
.chat-queue{display:flex;flex-direction:column;gap:4px;margin:0 0 8px}
.chat-queue>small{font-size:var(--text-xs);color:var(--muted);margin:0 2px}
.chat-queued{display:flex;align-items:center;gap:8px;padding:6px 6px 6px 8px;border:1px dashed var(--teal-line);border-radius:var(--radius-md);background:var(--teal-soft)}
.chat-queued-mark{flex-shrink:0;min-width:34px;padding:2px 6px;border-radius:var(--radius-sm);background:var(--surface);color:var(--teal);font-size:var(--text-xs);font-weight:600;text-align:center}
.chat-queued p{flex:1;min-width:0;margin:0;font-size:var(--text-md);color:var(--ink);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.chat-queued-files{font-size:var(--text-xs);color:var(--muted)}
.chat-queued button{flex-shrink:0;border:0;background:none;padding:3px 6px;border-radius:var(--radius-sm);font-size:var(--text-xs);color:var(--teal);cursor:pointer}
.chat-queued button:hover{background:var(--surface)}
.chat-queued .chat-queued-remove{color:var(--muted);font-size:var(--text-lg);line-height:1}
.chat-steer-send{margin-left:auto;background:var(--violet)}
.chat-steered{font-style:normal;font-size:var(--text-xs);padding:1px 6px;border-radius:var(--radius-sm);background:var(--violet-soft);color:var(--violet)}
.chat-steer-more{margin-left:2px}
.chat-steer-more :deep(.chip-menu-panel){min-width:240px}
.chat-steer-more nav button{display:flex;flex-direction:column;align-items:flex-start;gap:2px;width:100%;padding:7px 10px;border:0;border-radius:var(--radius-sm);background:none;text-align:left;cursor:pointer}
.chat-steer-more nav button:hover{background:var(--fill)}
.chat-steer-more nav strong{font-size:var(--text-sm);font-weight:550;color:var(--ink)}
.chat-steer-more nav small{font-size:var(--text-xs);color:var(--muted)}
.chat-steer-more nav button.danger strong{color:var(--danger-ink)}
.chat-steer-more+.chat-stop{margin-left:6px}

.chat-attached-images{display:flex;flex-wrap:wrap;gap:6px;margin:0 0 8px}
.chat-attached-images img{width:96px;height:72px;object-fit:cover;border-radius:var(--radius-md);border:1px solid var(--teal-line);background:var(--surface);cursor:zoom-in}

.chat-branch{max-width:min(30%,160px)}
.chat-branch :deep(.chip-menu-panel){min-width:220px;width:max-content;max-width:min(300px,78cqw);max-height:min(420px,66vh);overflow-y:auto;padding:5px}
/* Reads as the other chips' menus: a quiet heading, rows that lift on hover,
   and the current one in the selected tone rather than a colour of its own. */
.chat-branch-body header{padding:8px 10px 2px;font-size:var(--text-xs);font-weight:600;color:var(--ink-soft)}
.chat-branch-body p{font-size:var(--text-xs);line-height:1.6;color:var(--muted);margin:0 10px 6px;white-space:normal}
.chat-branch-body nav button{display:flex;align-items:center;justify-content:space-between;gap:8px;width:100%;min-height:36px;padding:8px 10px;border:0;border-radius:var(--radius-md);background:none;cursor:pointer;text-align:left}
.chat-branch-body nav button+button{margin-top:1px}
.chat-branch-body nav button:hover{background:var(--fill)}
.chat-branch-body nav button.selected{background:var(--selected)}
.chat-branch-body nav button.selected span{color:var(--accent-ink);font-weight:600}
.chat-branch-body nav span{font-size:var(--text-sm);color:var(--ink-soft);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.chat-branch-body nav small{flex-shrink:0;font-size:var(--text-xs);color:var(--muted)}
.chat-branch-body form{display:flex;gap:6px;margin:5px 3px 3px;padding-top:6px;border-top:1px solid var(--line)}
.chat-branch-body input{flex:1;min-width:0;height:30px;padding:0 8px;border:1px solid var(--line);border-radius:var(--radius-sm);font:inherit;font-size:var(--text-xs);background:var(--surface);color:var(--ink)}
.chat-branch-body form button{height:30px;padding:0 10px;border:0;border-radius:var(--radius-sm);background:var(--teal);color:var(--on-accent);font-size:var(--text-xs);cursor:pointer}
.chat-branch-body form button:disabled{opacity:.45;cursor:not-allowed}

/* The composer's controls sit in one row that scrolls sideways when the pane
   is too narrow for them, at any width, instead of squeezing until none is
   readable. Their menus are fixed to the window (ChipMenu), so the row's
   scrolling never clips them. */
.chat-chip-strip{display:flex;align-items:center;gap:6px;flex:1;min-width:0;overflow-x:auto;overflow-anchor:none;overscroll-behavior-x:contain;scrollbar-width:none;padding:2px 0}
.chat-chip-strip::-webkit-scrollbar{display:none}
.chat-chip-strip>*{flex:none}
@container(max-width:480px){
  .chat-chip-strip>*{max-width:150px!important}.chat-chip-strip>.chat-endpoint:has(.balance-badge){max-width:230px!important}
  .chat-composer footer{display:none}
  .chat-composer-controls>.chat-send{margin-left:0}
}

/* Phone composer: one rounded field, a line of text that grows as you type,
   and one row beneath it -- attach, settings, send -- like a phone chat app.
   The chips live in the settings sheet (.chip-rows) instead of this row. */
.chat-composer.is-phone{margin:0 10px 8px;padding:6px 6px max(6px,env(safe-area-inset-bottom));border-radius:var(--radius-xl)}
.chat-composer.is-phone>textarea{min-height:40px;max-height:38vh;field-sizing:content;resize:none;padding:8px 10px 4px;font-size:var(--input-text);line-height:1.5}
.chat-composer.is-phone .chat-composer-controls{gap:8px;align-items:center}
.chat-composer.is-phone .chat-chip-strip:not(.chip-rows),.chat-composer.is-phone footer{display:none}
.chat-composer.is-phone .chat-attach{width:40px;height:40px;min-width:40px;min-height:40px;border-radius:50%}
.chat-composer.is-phone .chat-send{width:40px;height:40px;min-width:40px;min-height:40px;border-radius:50%;font-size:var(--text-2xl)}
.chat-composer.is-phone .chat-send:not(.chat-steer-send){margin-left:auto}
.chat-options{display:flex;align-items:center;gap:6px;flex:0 1 auto;min-width:0;height:36px;padding:0 12px;border:0;border-radius:var(--radius-xl);background:var(--fill);color:var(--ink-soft);font:inherit;font-size:var(--text-md);cursor:pointer}
.chat-options>span{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.chat-options>svg{flex:none}
.chat-options-caret{transform:rotate(90deg);opacity:.6}
/* The summary names the model; tinting all of it red read as a model fault.
   The approval mode that stops asking keeps a mark of its own. */
.chat-options-danger{color:var(--danger)}
.chat-chip-strip.chip-rows{flex-direction:column;align-items:stretch;gap:2px;overflow:visible;padding:0}
.chat-chip-strip.chip-rows>*{max-width:none!important}
.chat-chip-strip.chip-rows>.chat-boot-chip{align-self:flex-start;margin:6px 12px}
/* A chip's panel shown as a sheet: the same content at phone reading size. */
.chip-menu-sheet nav button{min-height:52px;padding:10px 12px;border-radius:var(--radius-lg);font-size:var(--text-lg);white-space:normal}
.chip-menu-sheet nav button strong{font-size:var(--text-lg)}
.chip-menu-sheet nav button small{font-size:var(--text-md)}
.chip-menu-sheet .chat-effort-body{width:auto;max-width:none;padding:6px 12px 10px}
.chip-menu-sheet .chat-effort-body header strong,.chip-menu-sheet .chat-effort-body header em{font-size:var(--text-lg)}
.chip-menu-sheet .chat-effort-body footer{font-size:var(--text-sm)}
.chip-menu-sheet .chat-permission-body>header,.chip-menu-sheet .chat-branch-body p,.chip-menu-sheet .chat-model-hint{font-size:var(--text-md);margin-left:12px;margin-right:12px}
.chip-menu-sheet .chat-branch-body header{display:none}
.chip-menu-sheet .chat-branch-body nav span{font-size:var(--text-lg)}
.chip-menu-sheet .chat-branch-body input,.chip-menu-sheet .chat-model-custom input{height:44px;font-size:var(--input-text)}
.chip-menu-sheet .chat-branch-body form button,.chip-menu-sheet .chat-model-custom button{height:44px;padding:0 16px;font-size:var(--text-base)}
.chip-menu-sheet .chat-branch-body form,.chip-menu-sheet .chat-model-custom{margin:8px 12px 4px}

/* Phone reading: a conversation, not a document. Your messages are compact
   bubbles on the right without a "You" label, replies run full width without
   a header, and tool calls shrink to one quiet line each that opens on tap. */
.chat-pane.is-phone .chat-timeline{padding:16px 14px 12px}
.chat-pane.is-phone .chat-message{margin-bottom:18px}
.chat-pane.is-phone .chat-message :deep(.chat-markdown){font-size:var(--text-lg);line-height:1.7}
.chat-pane.is-phone .chat-message.user{max-width:86%;padding:9px 14px;border:0;border-radius:var(--radius-xl) var(--radius-xl) var(--radius-sm) var(--radius-xl);background:var(--fill)}
.chat-pane.is-phone .chat-message.assistant>.chat-message-label{display:none}
.chat-pane.is-phone .chat-message.user>.chat-message-label{margin-bottom:4px}
.chat-pane.is-phone .chat-message.user>.chat-message-label>span{display:none}
.chat-pane.is-phone .chat-message.user>.chat-message-label:not(:has(.chat-steered)){display:none}
.chat-pane.is-phone .chat-tool,.chat-pane.is-phone .chat-tool-run{margin:0 0 4px;border:0;border-radius:var(--radius-md);background:none}
.chat-pane.is-phone .chat-tool>summary,.chat-pane.is-phone .chat-tool-run>summary{min-height:36px;padding:6px 4px;gap:8px}
.chat-pane.is-phone .chat-tool>summary strong,.chat-pane.is-phone .chat-tool-run>summary strong{font-size:var(--text-md);font-weight:500;color:var(--ink-soft);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.chat-pane.is-phone .chat-tool-run>summary small{display:none}
.chat-pane.is-phone .chat-tool[open],.chat-pane.is-phone .chat-tool-run[open]{margin:4px 0 12px;background:var(--sunken)}
.chat-pane.is-phone .chat-tool[open]>summary,.chat-pane.is-phone .chat-tool-run[open]>summary{padding-left:10px;padding-right:10px}
.chat-pane.is-phone .chat-tool-run>.chat-tool{margin:0;border-radius:0}
.chat-pane.is-phone .chat-tool-run>.chat-tool>summary{padding-left:22px}
.chat-pane.is-phone .chat-tool pre{font-size:var(--text-sm);max-height:50vh}
.chat-pane.is-phone .chat-tool+.chat-message,.chat-pane.is-phone .chat-tool-solo+.chat-message,.chat-pane.is-phone .chat-tool-run+.chat-message{margin-top:10px}
/* Desktop conversation, lightened to match the phone: a compact user bubble
   with no label, tool calls as quiet rows that open into a recessed panel,
   and a composer that sits on the same column as the messages. */
.chat-pane:not(.is-phone) .chat-message.user{width:fit-content;max-width:min(82%,640px);padding:10px 16px;border:0;border-radius:var(--radius-xl) var(--radius-xl) var(--radius-sm) var(--radius-xl);background:var(--fill);box-shadow:inset 0 1px 0 var(--card-highlight)}
.chat-pane:not(.is-phone) .chat-message.user>.chat-message-label{margin-bottom:4px}
.chat-pane:not(.is-phone) .chat-message.user{margin-right:max(0px,calc((100% - 48rem) / 2))}
@media (hover:hover){.chat-pane:not(.is-phone) .chat-message-actions{opacity:0}.chat-pane:not(.is-phone) .chat-message:hover .chat-message-actions,.chat-pane:not(.is-phone) .chat-message-actions:focus-within{opacity:1}.chat-pane:not(.is-phone) .chat-message.user .chat-message-actions{position:absolute;right:4px;bottom:-26px;margin:0}.chat-pane:not(.is-phone) .chat-message.user{position:relative}}
.chat-pane:not(.is-phone) .chat-message.user>.chat-message-label>span{display:none}
.chat-pane:not(.is-phone) .chat-message.user>.chat-message-label:not(:has(.chat-steered)){display:none}
.chat-pane:not(.is-phone) .chat-tool,.chat-pane:not(.is-phone) .chat-tool-run{margin-bottom:6px;border:0;border-radius:var(--radius-md);background:none}
.chat-pane:not(.is-phone) .chat-tool>summary,.chat-pane:not(.is-phone) .chat-tool-run>summary{min-height:34px;padding:6px 8px;border-radius:var(--radius-md);transition:background var(--duration-fast)}
@media (hover:hover){.chat-pane:not(.is-phone) .chat-tool>summary:hover,.chat-pane:not(.is-phone) .chat-tool-run>summary:hover{background:var(--fill)}}
.chat-pane:not(.is-phone) .chat-tool>summary strong{font:var(--text-sm) var(--mono);color:var(--ink-soft);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.chat-pane:not(.is-phone) .chat-tool-run>summary strong{font-size:var(--text-sm);font-weight:500;color:var(--ink-soft)}
.chat-pane:not(.is-phone) .chat-tool[open],.chat-pane:not(.is-phone) .chat-tool-run[open]{margin:4px auto 12px;background:var(--sunken);box-shadow:inset 0 0 0 1px var(--border)}
.chat-pane:not(.is-phone) .chat-tool-run[open]>summary{border-bottom:1px solid var(--border);border-radius:var(--radius-md) var(--radius-md) 0 0}
.chat-pane:not(.is-phone) .chat-tool-run>.chat-tool{margin:0;box-shadow:none;border-radius:0}
.chat-pane:not(.is-phone) .chat-tool-run>.chat-tool>summary{padding-left:22px}
.chat-pane:not(.is-phone) .chat-tool+.chat-message,.chat-pane:not(.is-phone) .chat-tool-run+.chat-message{margin-top:4px}
.chat-composer:not(.is-phone){margin-inline:max(16px,calc((100% - 860px) / 2))}
.chat-composer:not(.is-phone) footer>span:first-child{opacity:0;transition:opacity var(--duration-normal)}
.chat-composer:not(.is-phone):focus-within footer>span:first-child{opacity:1}
.chat-native-note{display:flex;flex-wrap:wrap;justify-content:center;align-items:center;gap:var(--space-2) var(--space-4);padding:var(--space-4);border:0;border-radius:var(--radius-lg);background:var(--sunken);box-shadow:inset 0 0 0 1px var(--border)}
.chat-native-note>p{flex-basis:100%;margin:0;color:var(--ink-soft)}
.chat-pane:not(.is-phone) .chat-native-note>.chat-primary{min-height:36px;padding:0 16px;border-radius:var(--radius-md);font-weight:550}
.chat-approval-record{display:flex;align-items:center;gap:8px;max-width:48rem;margin:8px auto 14px;padding:6px 10px;border-radius:var(--radius-md);background:var(--sunken);font-size:var(--text-sm);color:var(--muted)}
.chat-approval-record>span{color:var(--ink-soft);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.chat-approval-record>small{margin-left:auto;flex:none;font-size:var(--text-xs)}
.chat-approval-record.pending{background:var(--warn-soft);color:var(--warn-ink)}
.chat-approval-record.pending>svg{color:var(--warn)}
/* min-height:0 + a non-zero shrink let the dock give up height in a short
   pane (the canvas tile is often shorter than 58vh); it then scrolls its own
   overflow instead of pushing the composer and the Submit footer off the
   clipped tile. The timeline (flex:1, basis 0) yields its height first. */
.chat-approval-dock{flex:0 1 auto;min-height:0;display:flex;flex-direction:column;gap:6px;max-height:min(58vh,560px);overflow:auto;padding:4px 6px 8px;margin:0 max(10px,calc((100% - 872px) / 2)) 2px;animation:dock-in var(--duration-normal) var(--ease-out)}
/* The card must not clip here: overflow!=visible would make the card its own
   scroll container and the sticky footer below would stick to the card (which
   never scrolls) instead of the dock. Re-round the accent bar it used to clip. */
.chat-approval-dock .chat-approval{flex-shrink:0;margin:0;max-width:none;overflow:visible;box-shadow:var(--shadow-md),var(--shadow-card)}
.chat-approval-dock .chat-approval::before{border-top-left-radius:var(--radius-lg);border-bottom-left-radius:var(--radius-lg)}
/* Pin the actions to the bottom of the scrolling dock so Submit stays in view
   however short the pane is. Opaque background hides content scrolling under it. */
.chat-approval-dock .chat-approval-actions{position:sticky;bottom:0;background:var(--surface);padding-bottom:var(--space-2)}
.chat-approval-pager{display:flex;align-items:center;gap:4px;padding:0 4px;font-size:var(--text-xs);color:var(--warn-ink)}
.chat-approval-pager>span{flex:1}
.chat-approval-pager .pager-back{transform:rotate(180deg)}
.chat-approval-key{margin-left:8px;padding:0 5px;border-radius:var(--radius-xs);background:color-mix(in srgb, var(--on-accent) 18%, transparent);font:var(--text-xs) var(--font-body)}
.chat-pane.is-phone .chat-approval-dock{margin:0 10px 8px;max-height:60vh}
@keyframes dock-in{from{opacity:0;transform:translateY(8px)}}
@media (prefers-reduced-motion:reduce){.chat-approval-dock{animation:none}}
.chat-approval-scope{display:flex;flex-wrap:wrap;align-items:center;gap:6px;margin-top:var(--space-2)}
.scope-access{padding:1px 8px;border-radius:var(--radius-full);background:var(--fill);color:var(--ink-soft);font-size:var(--text-xs);font-weight:600}
.scope-access.write,.scope-access.execute{background:var(--warn-soft);color:var(--warn-ink)}
.scope-access.network{background:var(--info-soft);color:var(--info-ink)}
.scope-outside{display:inline-flex;align-items:center;gap:4px;color:var(--danger-ink);font-size:var(--text-xs);font-weight:600}
.chat-approval-body .scope-path{padding:1px 6px;border-radius:var(--radius-xs);background:var(--danger-soft);color:var(--danger-ink);font-family:var(--mono);font-size:var(--text-xs);overflow-wrap:anywhere}
.scope-command{flex-basis:100%;margin:2px 0 0;padding:8px 10px;border-radius:var(--radius-md);background:var(--sunken);border:1px solid var(--border);font:var(--text-xs)/1.6 var(--mono);color:var(--ink);white-space:pre-wrap;overflow-wrap:anywhere;max-height:160px;overflow:auto}
.chat-approval-detail{margin-top:var(--space-2);font-size:var(--text-xs);color:var(--muted)}
.chat-approval-detail summary{cursor:pointer}
.chat-approval-detail pre{margin:6px 0 0;padding:8px 10px;border-radius:var(--radius-md);background:var(--sunken);font:var(--text-xs)/1.6 var(--mono);color:var(--ink-soft);white-space:pre-wrap;overflow-wrap:anywhere;max-height:200px;overflow:auto}
.chat-approval-actions button.allow-session{border-color:var(--accent);color:var(--accent-ink);background:var(--surface)}
@media (hover:hover){.chat-approval-actions button.allow-session:hover:not(:disabled){background:var(--accent-soft)}}
.chat-pane.is-phone .chat-approval-actions button.allow-session{grid-column:1/-1;order:-1}
/* Approval and question cards: a calm surface with one coloured voice (amber
   for a permission, teal for a question) on the icon and the left edge, so the
   request reads as part of the conversation rather than a warning banner. */
.chat-approval{--approval-tone:var(--warn);--approval-tone-soft:var(--warn-soft);--approval-tone-ink:var(--warn-ink);position:relative;max-width:48rem;margin:var(--space-4) auto;padding:var(--space-4) var(--space-4) var(--space-4) calc(var(--space-4) + 3px);background:var(--surface);border-radius:var(--radius-lg);box-shadow:var(--shadow-card);font-size:var(--text-md);color:var(--ink);overflow-wrap:anywhere;overflow:hidden}
.chat-approval::before{content:"";position:absolute;inset:0 auto 0 0;width:3px;background:var(--approval-tone)}
.chat-approval.is-question{--approval-tone:var(--accent);--approval-tone-soft:var(--accent-soft);--approval-tone-ink:var(--accent-ink)}
.chat-approval.resolved{--approval-tone:var(--line-strong);--approval-tone-soft:var(--fill);--approval-tone-ink:var(--muted);padding-block:var(--space-3);box-shadow:none;border:1px solid var(--border);background:var(--sunken)}
.chat-approval.resolved .chat-approval-body{display:none}
.chat-approval.folded{padding-block:var(--space-3)}
.chat-approval.folded .chat-approval-body,.chat-approval.folded .chat-question,.chat-approval.folded .chat-approval-actions,.chat-approval.folded .chat-approval-note{display:none}
.chat-approval.folded .chat-approval-heading strong{white-space:nowrap;overflow:hidden;text-overflow:ellipsis;font-size:var(--text-md)}
.chat-approval-fold{flex-shrink:0;color:var(--muted)}
.chat-approval-fold :deep(svg){transform:rotate(90deg);transition:transform var(--duration-fast) var(--ease-out)}
.chat-approval.folded .chat-approval-fold :deep(svg){transform:rotate(-90deg)}
.chat-approval-head{display:flex;align-items:center;gap:var(--space-3)}
.chat-approval-icon{display:grid;place-items:center;width:30px;height:30px;flex-shrink:0;border-radius:var(--radius-md);background:var(--approval-tone-soft);color:var(--approval-tone-ink)}
.chat-approval-heading{flex:1;min-width:0;display:flex;flex-direction:column;gap:2px}
.chat-approval-heading small{font-size:var(--text-xs);font-weight:600;letter-spacing:.02em;color:var(--approval-tone-ink)}
.chat-approval-heading strong{font-size:var(--text-lg);font-weight:600;line-height:1.35;color:var(--ink)}
.chat-approval.resolved .chat-approval-heading strong{font-size:var(--text-md);font-weight:500;color:var(--ink-soft)}
.chat-approval-state{flex-shrink:0;border-radius:var(--radius-full);background:var(--fill);color:var(--muted);font-size:var(--text-xs);padding:2px var(--space-2)}
.chat-approval-body{margin:var(--space-3) 0 0 calc(30px + var(--space-3));color:var(--ink-soft);line-height:1.7}
.chat-approval-body :deep(p){margin:0 0 var(--space-2)}
.chat-approval-body :deep(p:last-child){margin-bottom:0}
.chat-approval-body :deep(code){font-family:var(--mono);font-size:.92em;background:var(--fill);border-radius:var(--radius-sm);padding:1px 5px;color:var(--ink)}
.chat-approval-body :deep(pre){background:var(--sunken);border:1px solid var(--border);border-radius:var(--radius-md);padding:var(--space-3);margin:var(--space-2) 0;overflow:auto}
.chat-question{border:0;margin:var(--space-4) 0 0 calc(30px + var(--space-3));padding:0;min-width:0;display:flex;flex-direction:column;gap:var(--space-2)}
.chat-question legend{display:flex;align-items:baseline;flex-wrap:wrap;gap:var(--space-2);padding:0;margin-bottom:var(--space-2);font-size:var(--text-md);font-weight:600;line-height:1.5;color:var(--ink)}
.chat-question-tag{font-size:var(--text-xs);font-weight:600;color:var(--accent-ink);background:var(--accent-soft);border-radius:var(--radius-full);padding:1px var(--space-2)}
.chat-answers{display:grid;gap:6px}
.chat-answer{position:relative;display:flex;align-items:center;gap:var(--space-3);min-height:44px;padding:9px var(--space-3);border:1px solid var(--border);border-radius:var(--radius-md);background:var(--surface);cursor:pointer;transition:border-color var(--duration-fast),background var(--duration-fast)}
.chat-answer>input[type=radio],.chat-answer>input[type=checkbox]{position:absolute;opacity:0;width:1px;height:1px;pointer-events:none}
@media (hover:hover){.chat-answer:hover{background:var(--sunken);border-color:var(--line-strong)}}
.chat-answer:has(input:focus-visible){border-color:var(--accent);box-shadow:0 0 0 3px var(--accent-soft)}
.chat-answer.selected{border-color:var(--accent);background:var(--accent-soft)}
.chat-question:disabled .chat-answer{cursor:default;opacity:.7}
.chat-answer-marker{display:grid;place-items:center;width:16px;height:16px;flex-shrink:0;border:1.5px solid var(--line-strong);border-radius:var(--radius-full);color:var(--on-accent);background:var(--surface);transition:border-color var(--duration-fast),background var(--duration-fast),box-shadow var(--duration-fast)}
.chat-answer:has(input[type=checkbox]) .chat-answer-marker{border-radius:var(--radius-xs)}
.chat-answer:has(input[type=checkbox]) .chat-answer-marker svg{opacity:0}
.chat-answer.selected .chat-answer-marker{border-color:var(--accent);background:var(--accent)}
.chat-answer.selected:has(input[type=radio]) .chat-answer-marker{background:var(--surface);box-shadow:inset 0 0 0 3px var(--surface),inset 0 0 0 8px var(--accent)}
.chat-answer.selected:has(input[type=checkbox]) .chat-answer-marker svg{opacity:1}
.chat-answer-copy{min-width:0;display:flex;flex-direction:column;gap:2px}
.chat-answer-copy strong{font-size:var(--text-md);font-weight:500;line-height:1.4;color:var(--ink)}
.chat-answer-copy small{font-size:var(--text-sm);line-height:1.5;color:var(--ink-soft)}
.chat-answer-other{cursor:text}
.chat-answer-other .chat-answer-marker{border-style:dashed}
.chat-answer-other.selected .chat-answer-marker{border-style:solid;background:var(--surface);box-shadow:inset 0 0 0 3px var(--surface),inset 0 0 0 8px var(--accent)}
.chat-answer-other input{flex:1;min-width:0;border:0;outline:0;background:transparent;padding:0;font-size:var(--text-md);color:var(--ink)}
.chat-answer-other input::placeholder{color:var(--muted)}
/* The card shows the focus; a second ring on the field inside it doubles it. */
.chat-answer.chat-answer-other input:focus-visible{outline:0}
.chat-question-input{width:100%;min-width:0;min-height:44px;border:1px solid var(--line);border-radius:var(--radius-md);padding:9px var(--space-3);background:var(--surface);font-size:var(--text-md);color:var(--ink)}
.chat-question-input:focus{border-color:var(--accent);outline:0;box-shadow:0 0 0 3px var(--accent-soft)}
.chat-approval-actions{display:flex;flex-wrap:wrap;justify-content:flex-end;gap:var(--space-2);margin-top:var(--space-4);padding-top:var(--space-3);border-top:1px solid var(--border)}
.chat-approval-actions button{min-height:36px;padding:7px 14px;border-radius:var(--radius-md);border:1px solid var(--line);background:var(--surface);color:var(--ink-soft);font-size:var(--text-sm);font-weight:550;transition:background var(--duration-fast),border-color var(--duration-fast),color var(--duration-fast)}
.chat-approval-actions button.cancel{border-color:transparent;background:transparent;color:var(--muted)}
.chat-approval-actions button.deny{color:var(--danger-ink)}
.chat-approval-actions button.allow{background:var(--accent);border-color:var(--accent);color:var(--on-accent);box-shadow:var(--shadow-sm)}
@media (hover:hover){.chat-approval-actions button.cancel:hover:not(:disabled){background:var(--fill);color:var(--ink-soft)}.chat-approval-actions button.deny:hover:not(:disabled){background:var(--danger-soft);border-color:var(--danger-line)}.chat-approval-actions button.allow:hover:not(:disabled){background:var(--accent-hover);border-color:var(--accent-hover)}}
.chat-approval-note{margin:var(--space-3) 0 0 calc(30px + var(--space-3));font-size:var(--text-sm);color:var(--muted)}
.chat-pane.is-phone .chat-approval{padding:var(--space-3) var(--space-3) var(--space-3) calc(var(--space-3) + 3px)}
.chat-pane.is-phone .chat-approval-body,.chat-pane.is-phone .chat-question,.chat-pane.is-phone .chat-approval-note{margin-left:0}
.chat-pane.is-phone .chat-approval-actions{display:grid;grid-template-columns:1fr 1fr}
.chat-pane.is-phone .chat-approval-actions button{min-height:44px}
.chat-pane.is-phone .chat-answer-other input,.chat-pane.is-phone .chat-question-input{font-size:var(--input-text)}
.chat-pane.is-phone .chat-approval-actions button.allow{grid-column:1/-1;order:-1}
</style>

<style scoped>
.chat-options-alert{margin:0 12px 8px;border-radius:var(--radius-md);border:0}.chat-message-actions{display:flex;margin:0 0 0 -4px;opacity:.6;transition:opacity .15s}.chat-message.user .chat-message-actions{justify-content:flex-end;margin:4px -8px -8px 0}.chat-message:hover .chat-message-actions,.chat-message-actions:focus-within{opacity:1}
.chat-message :deep(.chat-markdown){-webkit-user-select:text;user-select:text;-webkit-touch-callout:default}
.chat-composer>textarea{-webkit-user-select:text;user-select:text;-webkit-touch-callout:default}
</style>
