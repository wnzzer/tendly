<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import type { Session } from "@tendly/protocol";
import { useI18n } from "../i18n";
import { isRemove, isRename } from "./shortcuts";
import { providerLabel } from "./api";
import { isEphemeralSession, isSessionArchived } from "./session-list";
import { workspaceSessionPane } from "./workspace-groups";
import Icon from "./Icon.vue";
import SessionCommonActions from "./SessionCommonActions.vue";
import SessionRunInfo from "./SessionRunInfo.vue";
import ProviderIcon from "./ProviderIcon.vue";

const props = withDefaults(defineProps<{
  session: Session;
  selected?: boolean;
  workspaceName?: string;
  archiveSupported?: boolean;
  archiveBusy?: boolean;
  keepBusy?: boolean;
  deleteBusy?: boolean;
  /** Shown with a checkbox, for acting on several sessions at once. */
  selectable?: boolean;
  checked?: boolean;
}>(), { selected: false, archiveSupported: false, archiveBusy: false, keepBusy: false, deleteBusy: false, selectable: false, checked: false });
const emit = defineEmits<{
  open: [session: Session];
  check: [session: Session, range: boolean];
  delete: [session: Session];
  environment: [id: string];
  rename: [session: Session, title: string];
  archive: [session: Session, archived: boolean];
  keep: [session: Session];
}>();
const { t } = useI18n();
const menuOpen = ref(false);
const showInfo = ref(false);
const menuButton = ref<HTMLButtonElement>();
const confirmDelete = ref(false);
/** Deleting is offered only where the server allows it: a temporary session,
 * or a permanent one already archived out of its workspace list. */
const deletable = computed(() => isEphemeralSession(props.session) || isSessionArchived(props.session));
const renaming = ref(false), titleDraft = ref(props.session.title), renameInput = ref<HTMLInputElement>();

/**
 * The provider icon already sits next to the title, so repeating "Claude Code"
 * here said nothing. Inside a workspace group the parent names the workspace,
 * leaving last-activity as the one genuinely useful detail for the row.
 */
const lastActive = computed(() => {
  const at = Date.parse(props.session.updated_at ?? "");
  if (!Number.isFinite(at)) return t("Not run yet");
  const minutes = Math.round((Date.now() - at) / 60000);
  if (minutes < 1) return t("just now");
  if (minutes < 60) return t("{n} min ago", { n: minutes });
  const hours = Math.round(minutes / 60);
  if (hours < 24) return t("{n} h ago", { n: hours });
  return t("{n} d ago", { n: Math.round(hours / 24) });
});
function dragSession(event: DragEvent) {
  event.dataTransfer?.setData("application/tendly-pane", JSON.stringify(workspaceSessionPane(props.session)));
  if (event.dataTransfer) event.dataTransfer.effectAllowed = "copyMove";
}
function toggleMenu() { menuOpen.value = !menuOpen.value; confirmDelete.value = false; }
/** Right-clicking a row opens the same menu its ··· button does, so the two
 * cannot drift into offering different actions. */
async function openMenuFromPointer(event: MouseEvent) {
  if (renaming.value) return;
  event.preventDefault();
  menuOpen.value = true; confirmDelete.value = false;
  // A right-click does not move focus, and this menu is dismissed by focus
  // leaving it. Focusing the button it belongs to restores that.
  await nextTick();
  menuButton.value?.focus();
}
async function closeMenu(restoreFocus = false) {
  menuOpen.value = false;
  if (restoreFocus) { await nextTick(); menuButton.value?.focus(); }
}
function escapeMenu(event: KeyboardEvent) {
  if (event.key !== "Escape" || !menuOpen.value) return;
  event.preventDefault(); event.stopPropagation(); void closeMenu(true);
}
function leaveMenu(event: FocusEvent) {
  if (event.relatedTarget instanceof Node && event.currentTarget instanceof Node && event.currentTarget.contains(event.relatedTarget)) return;
  menuOpen.value = false;
}
function archiveSession() {
  if (!props.archiveSupported || props.archiveBusy) return;
  emit("archive", props.session, !isSessionArchived(props.session));
  void closeMenu(true);
}
/**
 * Archiving, one gesture away. On a pointer it is a button beside ···; on a
 * touch screen the row slides left to show it, as phone lists do. Neither asks
 * first: archiving only hides the row, and the notice it leaves can undo it.
 */
const SWIPE_ACTION = 84;
const swipe = ref(0), swiping = ref(false), revealed = ref(false);
let touch: { x: number; y: number; from: number; decided: boolean } | undefined;
function touchStart(event: TouchEvent) {
  if (!props.archiveSupported || renaming.value || event.touches.length !== 1) return;
  const point = event.touches[0]!;
  touch = { x: point.clientX, y: point.clientY, from: revealed.value ? -SWIPE_ACTION : 0, decided: false };
}
function touchMove(event: TouchEvent) {
  if (!touch) return;
  const point = event.touches[0]!, dx = point.clientX - touch.x, dy = point.clientY - touch.y;
  // A scroll is a scroll: the row only follows a mostly sideways drag.
  if (!touch.decided) {
    if (Math.abs(dx) < 8 && Math.abs(dy) < 8) return;
    if (Math.abs(dy) > Math.abs(dx)) { touch = undefined; return; }
    touch.decided = true; swiping.value = true;
  }
  swipe.value = Math.max(-SWIPE_ACTION * 1.25, Math.min(0, touch.from + dx));
}
function touchEnd() {
  if (!touch) return;
  const moved = touch.decided; touch = undefined; swiping.value = false;
  if (!moved) return;
  revealed.value = swipe.value < -SWIPE_ACTION / 2;
  swipe.value = revealed.value ? -SWIPE_ACTION : 0;
}
function closeSwipe() { revealed.value = false; swipe.value = 0; }
/** A tap on a row that is slid open closes it rather than opening the session. */
function openRow() {
  if (revealed.value) { closeSwipe(); return; }
  emit("open", props.session);
}
function archiveFromRow() { closeSwipe(); archiveSession(); }
/**
 * The list's keys: ↑ ↓ Home End move between sessions, F2 renames, Delete
 * (⌘⌫ on a Mac) archives -- or, for one already archived, asks to delete it
 * -- and Space ticks it while choosing several.
 */
function rowKeydown(event: KeyboardEvent) {
  const rows = Array.from((event.currentTarget as HTMLElement).closest(".workspace-sidebar")?.querySelectorAll<HTMLElement>("[data-sidebar-row-focus]") ?? []);
  const index = rows.indexOf(event.currentTarget as HTMLElement);
  const move = event.key === "ArrowDown" ? index + 1 : event.key === "ArrowUp" ? index - 1 : event.key === "Home" ? 0 : event.key === "End" ? rows.length - 1 : undefined;
  if (move !== undefined && !event.altKey && !event.metaKey && !event.ctrlKey && !(props.selectable && event.shiftKey)) {
    event.preventDefault();
    rows[Math.max(0, Math.min(rows.length - 1, move))]?.focus();
    return;
  }
  // Choosing several, Shift+↑ ↓ extends the choice as Shift+click does.
  if (props.selectable && event.shiftKey && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
    event.preventDefault();
    const next = rows[index + (event.key === "ArrowDown" ? 1 : -1)];
    if (!next) return;
    if (!props.checked) emit("check", props.session, false);
    next.focus();
    next.closest(".sidebar-session-row")?.querySelector<HTMLInputElement>("input[type=checkbox]")?.dispatchEvent(new MouseEvent("click", { shiftKey: true, bubbles: true, cancelable: true }));
    return;
  }
  if (isRename(event)) { event.preventDefault(); beginRename(); return; }
  if (event.key === " " && props.selectable) { event.preventDefault(); emit("check", props.session, event.shiftKey); return; }
  if (!isRemove(event)) return;
  event.preventDefault();
  if (deletable.value && isSessionArchived(props.session)) { menuOpen.value = true; confirmDelete.value = true; return; }
  if (!props.archiveSupported || props.archiveBusy || isSessionArchived(props.session)) return;
  // The row leaves the list, so the next one takes the focus.
  (rows[index + 1] ?? rows[index - 1])?.focus();
  archiveFromRow();
}
function deleteSession() {
  if (!deletable.value || props.deleteBusy) return;
  emit("delete", props.session); confirmDelete.value = false; void closeMenu(true);
}
function environment() { emit("environment", props.session.id); void closeMenu(true); }
function keepSession() {
  if (!isEphemeralSession(props.session) || props.keepBusy) return;
  emit("keep", props.session); void closeMenu(true);
}
function beginRename() {
  titleDraft.value = props.session.title;
  menuOpen.value = false;
  renaming.value = true;
  void nextTick(() => { renameInput.value?.focus(); renameInput.value?.select(); });
}
const rowButton = ref<HTMLButtonElement>();
/** Either way out of the name goes back to the row, where F2 came from. */
function cancelRename() { renaming.value = false; titleDraft.value = props.session.title; void nextTick(() => rowButton.value?.focus()); }
function submitRename() {
  const title = titleDraft.value.trim();
  if (!title) { renameInput.value?.focus(); return; }
  renaming.value = false;
  emit("rename", props.session, title);
  void nextTick(() => rowButton.value?.focus());
}
</script>

<template>
  <li class="sidebar-session-row" :class="{ selected, checked, archived: isSessionArchived(session), swiped: swipe !== 0 }" :data-sidebar-session-id="session.id" @contextmenu="openMenuFromPointer" @keydown="escapeMenu" @focusout="leaveMenu">
    <button v-if="archiveSupported" type="button" class="session-swipe-action" :tabindex="revealed ? 0 : -1" :aria-hidden="!revealed" :disabled="archiveBusy" @click="archiveFromRow"><Icon :name="isSessionArchived(session) ? 'restore' : 'archive'" :size="17" />{{ t(isSessionArchived(session) ? 'Restore' : 'Archive') }}</button>
    <div class="sidebar-session-main" :class="{ 'is-swiping': swiping }" :style="swipe ? { transform: `translateX(${swipe}px)` } : undefined" @touchstart.passive="touchStart" @touchmove.passive="touchMove" @touchend="touchEnd" @touchcancel="touchEnd">
      <label v-if="selectable" class="sidebar-session-check" :title="t('Select session')" @click.stop><input type="checkbox" :checked="checked" :aria-label="t('Select {session}', { session: session.title })" @click.prevent="emit('check', session, $event.shiftKey)" /></label>
      <button ref="rowButton" class="workspace-session" data-sidebar-row-focus :class="{ active: selected }" @keydown="rowKeydown" :aria-current="selected ? 'true' : undefined" :title="`${session.title}\n${workspaceName ? `${workspaceName} · ` : ''}${providerLabel(session.provider)} · ${t(session.status)}`" draggable="true" @dragstart="dragSession" @click="openRow">
        <span :class="['workspace-session-provider', session.provider]"><ProviderIcon :provider="session.provider" :size="14" /></span>
        <span class="workspace-session-text"><strong class="workspace-session-title">{{ session.title }}</strong><small v-if="session.preview" class="workspace-session-preview">{{ session.preview }}</small></span>
        <small class="workspace-session-meta"><Icon v-if="isSessionArchived(session)" name="archive" :size="11" :aria-label="t('Archived session')" /><span v-if="isEphemeralSession(session)" class="session-temporary-badge">{{ t('Temporary') }}</span><span v-if="session.status === 'waiting'" class="session-waiting-badge">{{ t('Needs you') }}</span><span v-else-if="session.status !== 'stopped'" :class="['state-dot', session.status]" :title="t(session.status)" :aria-label="t(session.status)" /><span class="workspace-session-when">{{ workspaceName || lastActive }}</span></small>
      </button>
      <div class="sidebar-session-actions">
        <button v-if="archiveSupported" class="icon-button session-archive-quick" :disabled="archiveBusy" :aria-label="t(isSessionArchived(session) ? 'Restore {session}' : 'Archive {session}', { session: session.title })" :title="t(isSessionArchived(session) ? 'Restore this session to its workspace list.' : 'Archive hides this session from workspace lists without stopping it.')" @click.stop="archiveSession"><Icon :name="isSessionArchived(session) ? 'restore' : 'archive'" :size="14" /></button>
        <button ref="menuButton" class="icon-button session-more-button" :class="{ selected: menuOpen }" :aria-label="t('Session actions for {session}', { session: session.title })" :aria-expanded="menuOpen" :title="t('Session actions')" @click="toggleMenu"><Icon name="more" :size="15" /></button>
      </div>
    </div>
    <form v-if="renaming" class="sidebar-session-rename" @submit.prevent="submitRename" @keydown.esc.prevent.stop="cancelRename">
      <input ref="renameInput" v-model="titleDraft" :aria-label="t('Session title')" :placeholder="t('Session title')" maxlength="200" />
      <button type="submit" :aria-label="t('Save session name')" :title="t('Save session name')"><Icon name="check" :size="12" /></button>
      <button type="button" :aria-label="t('Cancel rename')" :title="t('Cancel rename')" @click="cancelRename"><Icon name="close" :size="12" /></button>
    </form>
    <div v-else-if="menuOpen" class="sidebar-session-menu" role="menu" :aria-label="t('Session actions')">
      <SessionCommonActions :session="session" :keep-busy="keepBusy" :close-menu="() => closeMenu(true)" @rename="beginRename" @environment="environment" @keep="keepSession" @info="showInfo = true" />
      <hr />
      <button class="session-archive-action" role="menuitem" :disabled="!archiveSupported || archiveBusy" :aria-busy="archiveBusy" :title="!archiveSupported ? t('Upgrade the backend to archive sessions.') : t(isSessionArchived(session) ? 'Restore this session to its workspace list.' : 'Archive hides this session from workspace lists without stopping it.')" @click="archiveSession"><Icon :name="isSessionArchived(session) ? 'restore' : 'archive'" :size="13" />{{ t(archiveBusy ? 'Saving…' : isSessionArchived(session) ? 'Restore session' : 'Archive session') }}</button>
      <small v-if="!archiveSupported">{{ t('Upgrade the backend to archive sessions.') }}</small>
      <template v-if="deletable">
        <template v-if="confirmDelete">
          <p class="session-delete-confirm">{{ t('Delete {session}? Its record and conversation are removed and cannot be restored.', { session: session.title }) }}</p>
          <button class="session-delete-action" role="menuitem" :disabled="deleteBusy" :aria-busy="deleteBusy" @click="deleteSession"><Icon name="close" :size="13" />{{ t(deleteBusy ? 'Deleting…' : 'Delete permanently') }}</button>
          <button role="menuitem" @click="confirmDelete = false">{{ t('Cancel') }}</button>
        </template>
        <button v-else class="session-delete-action" role="menuitem" :disabled="deleteBusy" @click="confirmDelete = true"><Icon name="close" :size="13" />{{ t('Delete session…') }}</button>
      </template>
    </div>
    <Teleport to="body"><SessionRunInfo v-if="showInfo" :session="session" @close="showInfo = false" /></Teleport>
  </li>
</template>

<style scoped>
.sidebar-session-row{min-width:0;list-style:none;scroll-margin-block:14px;border-radius:var(--radius-sm)}.sidebar-session-main{display:flex;align-items:center;min-width:0}.sidebar-session-main{position:relative}.workspace-session{display:flex;align-items:center;gap:8px;flex:1;min-width:0;border:0;border-radius:var(--radius-sm);padding:7px 8px;margin:1px 0;background:none;text-align:left}.workspace-session:hover{background:var(--fill)}.workspace-session.active{background:var(--selected)}.workspace-session-provider{display:grid;place-items:center;flex-shrink:0;width:16px;color:#c0835a}.workspace-session-provider.codex{color:var(--violet)}.workspace-session-provider.terminal{color:var(--muted)}.workspace-session-title{flex:1;min-width:0;font-size:var(--text-sm);font-weight:500;color:var(--ink);line-height:18px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.workspace-session.active .workspace-session-title{color:var(--ink);font-weight:600}.workspace-session-meta{display:flex;align-items:center;gap:5px;flex:none;max-width:45%;font-size:var(--text-xs);line-height:16px;color:var(--faint);white-space:nowrap}.workspace-session-when{overflow:hidden;text-overflow:ellipsis}.workspace-session-meta>svg{flex:none;color:#ad8d60}.workspace-session-meta>.state-dot{width:7px;height:7px}.sidebar-session-actions{position:absolute;right:3px;top:50%;transform:translateY(-50%);display:flex;align-items:center;gap:1px;opacity:0;pointer-events:none;border-radius:var(--radius-sm);background:var(--fill)}.sidebar-session-row:hover .sidebar-session-actions,.sidebar-session-main:focus-within .sidebar-session-actions,.sidebar-session-actions:has(.selected){opacity:1;pointer-events:auto}.sidebar-session-row:hover .workspace-session-meta,.sidebar-session-main:focus-within .workspace-session-meta{visibility:hidden}.sidebar-session-row.selected .sidebar-session-actions{background:var(--selected)}.sidebar-session-actions>.icon-button{width:26px;height:26px;padding:4px}.sidebar-session-check{display:grid;place-items:center;flex:none;width:20px;height:28px;cursor:pointer}.sidebar-session-check>input{width:12px;height:12px;margin:0;accent-color:var(--teal);cursor:pointer}.sidebar-session-row.checked>.sidebar-session-main>.workspace-session{background:var(--accent-soft)}.session-more-button{font-size:var(--text-xl);line-height:18px;color:var(--muted)}.session-more-button:hover,.session-more-button.selected{color:var(--ink);background:var(--fill)}.sidebar-session-menu{margin:0 2px 6px 27px;padding:4px;border:1px solid var(--fill-hover);border-radius:var(--radius-sm);background:var(--surface)}.sidebar-session-menu>button{display:flex;align-items:center;gap:6px;width:100%;border:0;border-radius:var(--radius-xs);background:none;padding:6px 5px;text-align:left;font-size:var(--text-xs);line-height:15px;color:var(--ink-soft)}.sidebar-session-menu>button>svg{flex:none}.sidebar-session-menu>button:hover:not(:disabled){background:var(--fill);color:var(--ink)}.sidebar-session-menu>button:disabled{opacity:.55;cursor:not-allowed}.session-temporary-badge{flex:none;border-radius:var(--radius-xs);padding:0 4px;background:var(--violet-soft);color:var(--violet);font-size:var(--text-xs);line-height:13px}.session-keep-action>svg{color:var(--violet)}.session-environment-action>svg{color:var(--info)}.session-archive-action>svg{color:#ad8d60}.archived .session-archive-action>svg{color:var(--ok-ink)}.sidebar-session-menu>hr{border:0;border-top:1px solid var(--fill);margin:3px 4px}.session-delete-action{color:var(--danger)!important}.session-delete-action>svg{color:var(--danger)}.session-delete-action:hover:not(:disabled){background:var(--danger-soft)!important}.session-delete-confirm{margin:2px 5px 3px;font-size:var(--text-xs);line-height:13px;color:var(--danger-ink)}.sidebar-session-menu>small{display:block;padding:2px 5px;font-size:var(--text-xs);line-height:14px;color:#9c875f}
.session-rename-action>svg{color:var(--info)}.sidebar-session-rename{display:flex;align-items:center;gap:4px;margin:2px 2px 6px 27px;padding:3px;border:1px solid var(--ok-line);border-radius:var(--radius-sm);background:var(--sunken)}.sidebar-session-rename input{min-width:0;flex:1;border:0;background:transparent;padding:5px 4px;font-size:var(--text-xs);color:var(--ink-soft);outline:0}.sidebar-session-rename button{display:grid;place-items:center;width:26px;height:26px;border:0;border-radius:var(--radius-sm);background:none;color:var(--ink-soft);cursor:pointer}.sidebar-session-rename button:first-of-type{color:var(--accent-ink)}.sidebar-session-rename button:hover{background:var(--accent-soft)}
@media(pointer:coarse){.workspace-session{min-height:44px}.sidebar-session-actions>.icon-button{width:34px;height:44px}.sidebar-session-menu>button{min-height:44px;font-size:var(--text-sm)}.sidebar-session-rename{min-height:44px}.sidebar-session-rename input{font-size:var(--input-text)}.sidebar-session-rename button{width:36px;height:36px}.sidebar-session-actions{position:static;transform:none;opacity:1;pointer-events:auto;background:none}.sidebar-session-row:hover .workspace-session-meta,.sidebar-session-main:focus-within .workspace-session-meta{visibility:visible}.workspace-session-title{font-size:var(--text-lg)}.workspace-session-meta{font-size:var(--text-sm)}}
/* Title over the last thing said, so rows that share a default name differ. */
.workspace-session-text{display:flex;flex-direction:column;flex:1;min-width:0}
.workspace-session-text>.workspace-session-title{flex:none}
.workspace-session-preview{display:none;margin-top:2px;font-size:var(--text-xs);line-height:15px;color:var(--muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.session-archive-quick>svg{color:#ad8d60}
.sidebar-session-row{position:relative;overflow:hidden}
.sidebar-session-main{transition:transform var(--duration-fast) var(--ease-out)}
/* Only while slid: the row covers the action behind it in the sidebar's own colour. */
.sidebar-session-row.swiped>.sidebar-session-main{background:var(--sunken)}
.sidebar-session-main.is-swiping{transition:none}
.session-swipe-action{position:absolute;top:1px;bottom:1px;right:0;width:84px;display:none;flex-direction:column;align-items:center;justify-content:center;gap:3px;border:0;border-radius:var(--radius-sm);background:var(--warn-soft);color:var(--warn-ink);font:inherit;font-size:var(--text-xs);font-weight:600;cursor:pointer}
.sidebar-session-row.swiped>.session-swipe-action{display:flex}
.archived>.session-swipe-action{background:var(--ok-soft);color:var(--ok-ink)}
@media(pointer:coarse){
  .workspace-session-preview{display:block;font-size:var(--text-sm);line-height:18px}
  .workspace-session-text>.workspace-session-title{white-space:normal;display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:2;line-height:22px}
  .workspace-session{align-items:flex-start;padding-block:9px}
  .workspace-session-provider,.workspace-session-meta{margin-top:3px}
  .sidebar-session-actions{align-self:flex-start;margin-top:2px}
  /* A swipe archives here; a second button on every row would only crowd it. */
  .session-archive-quick{display:none}
}
.session-waiting-badge{flex:none;padding:0 6px;border-radius:var(--radius-full);background:var(--warn-soft);color:var(--warn-ink);font-size:var(--text-xs);font-weight:600;line-height:18px}
</style>
