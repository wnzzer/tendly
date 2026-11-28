<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import ContextMenu from "../features/ContextMenu.vue";
import Icon from "../features/Icon.vue";
import TerminalKeyBar from "./TerminalKeyBar.vue";
import TerminalSearchBar from "./TerminalSearchBar.vue";
import TerminalUploads from "./TerminalUploads.vue";
import { useI18n } from "../i18n";
import { copyText, preserveNativeContextMenu } from "../features/clipboard";
import {
  createSessionStream,
  PING_MESSAGE,
  TERMINAL_VIEW_ERROR,
  type SessionStreamState,
} from "../features/session-stream";
import { onPageReturn } from "../features/page-return";
import {
  arrowSequence,
  ctrlSequence,
  ENTER_SEQUENCE,
  isTap,
  keyClient,
  repeatArrow,
  selectionPresses,
  shiftSequence,
  tabSequence,
  type ArrowKey,
} from "../features/terminal-keys";
import { terminalDark as dark } from "../features/theme";
import { textScale } from "../features/appearance";
import { terminalEngine } from "../features/terminal-engine";
import type { TerminalLinkTarget } from "../features/terminal-links";
import {
  clearsSelectionOnCopy,
  findCount,
  isTerminalFind,
  terminalClipboardKey,
  terminalFindChord,
} from "../features/terminal-search";
import { formatChord } from "../features/shortcuts";
import { filesFrom, useTerminalUploads } from "../features/terminal-uploads";
import { showToast } from "../features/toasts";
import { terminalTheme, type FindState, type TerminalView, type TerminalViewEvents } from "../features/terminal-view";
import { afterHello, parseControl, readFramed, streamUrl, type ResumePoint } from "../features/terminal-protocol";
import { createWtermView } from "../features/terminal-view-wterm";
import { createXtermView } from "../features/terminal-view-xterm";
import { createZmodem, type ZmodemStatus } from "../features/zmodem";
const { t } = useI18n();

/**
 * A native session's terminal: the PTY stream on one side, a terminal view on
 * the other, and the controls a phone needs in between. Which library draws
 * is chosen once per browser (`terminal-engine.ts`); the pane talks to either
 * through `TerminalView` and owns everything around it.
 */
/** `provider` is the program the session runs, which decides the keys on the bar. */
const props = defineProps<{ sessionId: string; provider?: string }>();
const emit = defineEmits<{
  exit: [];
  status: [connected: boolean];
  openFile: [reference: { path: string; line?: number; cwd?: string }];
}>();
const host = ref<HTMLDivElement>();
const state = ref<SessionStreamState>("connecting");
const notice = ref("");
const nativeNotice = ref(false);
/**
 * On-screen keys, for a device whose keyboard has no arrows and cannot be
 * relied on to be visible at all. A physical keyboard already sends these, so
 * the controls appear only where the pointer is a fingertip.
 */
const touchDevice = ref(false);
/**
 * Tap a row to move a list selection onto it.
 *
 * Off by default and switched on explicitly, because the same tap means two
 * different things depending on what is running. In a menu it picks a row; at a
 * shell prompt the arrows it sends walk through command history instead, which
 * is not something a stray tap should do. The terminal cannot tell those apart
 * -- the client command this exists for draws its menu in the ordinary buffer,
 * exactly like a prompt -- so the person holding the phone says which it is.
 */
const tapSelect = ref(false);
const ctrlArmed = ref(false);
const shiftArmed = ref(false);
const moreKeys = ref(false);
/** Whether the view is scrolled back from the newest output, which offers a way back down. */
const scrolledBack = ref(false);
/**
 * Find in the terminal's whole history, not only the rows on screen: the
 * browser's own find sees just the rows wterm has mounted around the view.
 * xterm.js offers no find here, so the button shows only where the view can.
 */
const searchOpen = ref(false);
const searchable = ref(false);
const searchQuery = ref("");
const searchProgress = ref<FindState>();
const searchBar = ref<InstanceType<typeof TerminalSearchBar>>();
const searchCount = computed(() => findCount(searchQuery.value, searchProgress.value, t("No matches")));
const findShortcut = formatChord(terminalFindChord());
/**
 * Text waiting to be pasted, when the clipboard could not be read directly.
 * `undefined` means the paste field is closed.
 */
const pasteDraft = ref<string>();
const pasteField = ref<HTMLTextAreaElement>();
const fileInput = ref<HTMLInputElement>();
const rzFileInput = ref<HTMLInputElement>();
const zmodemStatus = ref<ZmodemStatus>({ phase: "idle" });
const pendingOscCopy = ref<string>();
const contextMenu = ref<{
  x: number;
  y: number;
  selectionText: string;
  mouseTracking: boolean;
  link?: TerminalLinkTarget;
}>();
let rightClickSelection = "";

const engine = terminalEngine();
let view: TerminalView | undefined;
let terminalCwd: string | undefined;
let observer: ResizeObserver | undefined;
let resizeFrame = 0,
  replayTimer = 0,
  replaying = false,
  sized = false,
  disposed = false;
/**
 * Where a reconnect picks up. The server resumes from there when it still can,
 * so a dropped socket costs nothing on screen; otherwise it says so and the
 * view is redrawn from the replay. `framed` is whether this socket numbers its
 * frames; `redrawPending` holds the old screen until a server that does not
 * says what it is replaying.
 */
let resume: ResumePoint | undefined;
let framed = false,
  redrawPending = false;

function triggerDownload(name: string, bytes: Uint8Array) {
  const blob = new Blob([bytes as BlobPart], { type: "application/octet-stream" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  anchor.style.display = "none";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
  showToast(t("Downloaded {name}", { name }));
}

const zmodem = createZmodem({
  send: bytes => {
    stream.send(bytes);
  },
  onDownload: triggerDownload,
  onStatus(next) {
    zmodemStatus.value = next;
  },
  onError(message) {
    showToast(t(message), { tone: "warn" });
  },
});

const stream = createSessionStream({
  createSocket: url => new WebSocket(url),
  reconnect: true,
  hidden: () => document.hidden,
  onState(next, message) {
    state.value = next;
    notice.value = message;
    nativeNotice.value = false;
    if (next !== "connected") zmodem.reset();
    emit("status", next === "connected");
  },
  // Until the server says whether it resumed, the old screen stays up rather
  // than blanking for the length of a handshake.
  onOpen() {
    sized = false;
    framed = false;
    redrawPending = true;
    replaying = true;
    window.clearTimeout(replayTimer);
    resize();
  },
  onPong() {
    replaying = false;
    window.clearTimeout(replayTimer);
  },
  onMessage(data) {
    if (data instanceof ArrayBuffer) {
      let bytes: Uint8Array;
      if (framed) {
        const frame = readFramed(data);
        if (!frame) return;
        resume = { runtime: resume?.runtime ?? "", seq: frame.seq };
        bytes = frame.bytes;
      } else {
        // A server that does not number its frames replays everything.
        if (redrawPending) {
          redrawPending = false;
          view?.reset();
          window.clearTimeout(replayTimer);
          replayTimer = window.setTimeout(() => {
            replaying = false;
          }, 300);
        }
        bytes = new Uint8Array(data);
      }
      const visible = zmodem.consume(bytes, replaying);
      if (visible.byteLength) view?.write(visible);
      // A size sent as the socket opens can reach the server before a reopened
      // session's process exists, leaving the shell at the size it started
      // with. Its first output proves the process is there; say the size again.
      if (!sized) {
        sized = true;
        resize();
      }
      return;
    }
    if (typeof data !== "string") return;
    const control = parseControl(data);
    if (!control) {
      nativeNotice.value = false;
      notice.value = "An unsupported control message was received. Reconnect to refresh the view.";
      return;
    }
    const message = typeof control.message === "string" ? control.message : undefined;
    if (control.type === "stream") {
      framed = true;
      redrawPending = false;
      const next = afterHello(control, resume);
      if (next.reset) {
        view?.reset();
        replaying = true;
        stream.send(PING_MESSAGE);
        window.clearTimeout(replayTimer);
        replayTimer = window.setTimeout(() => {
          replaying = false;
        }, 1500);
      } else {
        replaying = false;
        window.clearTimeout(replayTimer);
      }
      resume = next.resume;
    } else if (control.type === "exit") {
      // The shell around this pane says the process exited and offers to start it again.
      stream.finish("");
      emit("exit");
    } else if (control.type === "gap") {
      nativeNotice.value = message !== undefined;
      notice.value =
        message ??
        "The stream exceeded its buffer. Reconnect to restore available history; this does not restart the process.";
    } else if (control.type === "error") {
      stream.fail(message ?? "The session stream reported an error.");
      nativeNotice.value = message !== undefined;
    }
  },
});

/** What the view reports back. */
const viewEvents: TerminalViewEvents = {
  /** What the keyboard typed, with any armed on-screen modifier applied. */
  typed(data) {
    replaying = false;
    if (shiftArmed.value) {
      shiftArmed.value = false;
      data = shiftSequence(data);
    }
    if (ctrlArmed.value) {
      ctrlArmed.value = false;
      data = ctrlSequence(data);
    }
    stream.send(new TextEncoder().encode(data));
  },
  send: bytes => stream.send(bytes),
  scrolled: updateScrolledBack,
  copy(text) {
    if (!replaying && text) {
      void copyText(text).then(
        () => {
          pendingOscCopy.value = undefined;
        },
        () => {
          // Plain HTTP LAN/Tailscale rejects async clipboard writes without a click gesture.
          pendingOscCopy.value = text;
        },
      );
    }
  },
  cwdChanged(cwd) {
    terminalCwd = cwd;
  },
  openLink(link) {
    activateLink(link);
  },
  ready() {
    searchable.value = !!view?.search;
    if (document.activeElement === host.value) view?.focus();
    resize();
  },
  failed() {
    stream.fail(TERMINAL_VIEW_ERROR);
    cleanupView();
  },
  searched(progress) {
    searchProgress.value = progress;
  },
  // Before it is ready the grid is still being built; `ready` sends the size.
  resized(size) {
    if (view?.ready) sendSize(size);
  },
};

function resize() {
  cancelAnimationFrame(resizeFrame);
  resizeFrame = requestAnimationFrame(() => {
    if (disposed || !view?.ready || !host.value || host.value.clientWidth < 30 || host.value.clientHeight < 30) return;
    try {
      view.fit();
    } catch {
      return;
    }
    const size = view.size();
    if (size) sendSize(size);
  });
}
function sendSize(size: { cols: number; rows: number }) {
  stream.send(JSON.stringify({ type: "resize", ...size }));
}

/** Send exactly the bytes the physical key would, so the client sees a key. */
function sendKeys(sequence: string) {
  if (!sequence || !view?.ready) return;
  // A key on the bar is the key the modifiers were armed for, if any; they are
  // released rather than left waiting for the keyboard.
  ctrlArmed.value = false;
  shiftArmed.value = false;
  stream.send(new TextEncoder().encode(sequence));
}
/**
 * A key on the bar that is more than one press, such as Esc twice: each press
 * is its own write, a moment apart, as a hand would make them.
 */
async function sendPresses(presses: readonly string[]) {
  for (const [index, press] of presses.entries()) {
    if (index) await new Promise(done => setTimeout(done, 60));
    sendKeys(press);
  }
}
function pressArrow(key: ArrowKey) {
  if (!view?.ready) return;
  sendKeys(arrowSequence(key, view.applicationCursorKeys(), { shift: shiftArmed.value, ctrl: ctrlArmed.value }));
}
function focusView() {
  view?.focus();
}
/**
 * Paste from the clipboard.
 *
 * The Clipboard API is available only in a secure context, and a phone often
 * reaches Tendly over plain http on a private network. Where it cannot be
 * read -- or the browser refuses -- an ordinary field opens instead, which
 * every mobile browser can paste into with a long press.
 */
async function pressPaste() {
  ctrlArmed.value = false;
  shiftArmed.value = false;
  try {
    // An image on the clipboard first, where the browser lets it be read.
    const items = await navigator.clipboard?.read?.().catch(() => undefined);
    const images: File[] = [];
    for (const item of items ?? []) {
      const type = item.types.find(kind => kind.startsWith("image/"));
      if (type) images.push(new File([await item.getType(type)], "image." + (type.split("/")[1] ?? "png"), { type }));
    }
    if (images.length) {
      void files.sendFiles(images);
      return;
    }
    const text = await navigator.clipboard?.readText?.();
    if (text) {
      view?.paste(text);
      return;
    }
  } catch {
    /* Not permitted here; fall back to the field. */
  }
  pasteDraft.value = "";
  await nextTick();
  pasteField.value?.focus();
}

const files = useTerminalUploads({
  sessionId: () => props.sessionId,
  canUpload: () => !!view?.ready && state.value === "connected",
  paste: text => view?.paste(text),
  focus: focusView,
  translate: text => t(text),
  warn: message => showToast(message, { tone: "warn" }),
});
const { uploads, dragging } = files;
function pickFiles() {
  moreKeys.value = false;
  fileInput.value?.click();
}
function filesPicked(event: Event) {
  const input = event.target as HTMLInputElement;
  const picked = [...(input.files ?? [])];
  input.value = "";
  void files.sendFiles(picked);
}
function pickRzFiles() {
  rzFileInput.value?.click();
}
function rzFilesPicked(event: Event) {
  const input = event.target as HTMLInputElement;
  const picked = [...(input.files ?? [])];
  input.value = "";
  zmodem.sendFiles(picked);
  focusView();
}
function cancelZmodem() {
  zmodem.cancel();
  focusView();
}
async function flushPendingOscCopy() {
  const text = pendingOscCopy.value;
  pendingOscCopy.value = undefined;
  if (text) await copyText(text).catch(() => undefined);
  focusView();
}
function formatTransferProgress(transferred: number, total?: number): string {
  if (total && total > 0) return `${Math.min(100, Math.round((transferred / total) * 100))}%`;
  if (transferred < 1024) return `${transferred} B`;
  if (transferred < 1024 * 1024) return `${(transferred / 1024).toFixed(1)} KiB`;
  return `${(transferred / (1024 * 1024)).toFixed(1)} MiB`;
}
function activateLink(link: TerminalLinkTarget) {
  if (link.kind === "url") {
    window.open(link.href, "_blank", "noopener,noreferrer");
    return;
  }
  emit("openFile", { path: link.path, line: link.line, cwd: terminalCwd });
}
async function copySelection(clearAfter = false) {
  const text = view?.getSelection() || window.getSelection()?.toString() || "";
  if (text) {
    await copyText(text).catch(() => undefined);
    if (clearAfter) {
      view?.clearSelection();
      window.getSelection()?.removeAllRanges();
    }
    return;
  }
  document.execCommand("copy");
}
function openContextMenu(event: MouseEvent) {
  if (preserveNativeContextMenu(event, false)) return;
  event.preventDefault();
  event.stopPropagation();
  const selectionText = view?.getSelection() || window.getSelection()?.toString() || rightClickSelection;
  rightClickSelection = "";
  contextMenu.value = {
    x: event.clientX,
    y: event.clientY,
    selectionText,
    mouseTracking: !!view?.mouseTracking(),
    link: view?.linkAt?.(event.clientX, event.clientY, event.target),
  };
}
function menuOpenLink() {
  const link = contextMenu.value?.link;
  contextMenu.value = undefined;
  if (link) activateLink(link);
}
function menuCopy() {
  const text = contextMenu.value?.selectionText || "";
  contextMenu.value = undefined;
  if (text) void copyText(text).catch(() => undefined);
  else void copySelection(false);
  focusView();
}
function menuPaste() {
  contextMenu.value = undefined;
  void pressPaste();
}
async function menuSelectAll() {
  contextMenu.value = undefined;
  await view?.selectAll();
}
function menuFind() {
  contextMenu.value = undefined;
  openSearch();
}
/** The phone paste field takes images too: a long-press paste of a photo lands here. */
function onFieldPaste(event: ClipboardEvent) {
  const pasted = filesFrom(event.clipboardData);
  if (!pasted.length) return;
  event.preventDefault();
  pasteDraft.value = undefined;
  void files.sendFiles(pasted);
}
function sendPaste() {
  const text = pasteDraft.value;
  pasteDraft.value = undefined;
  if (text) view?.paste(text);
  focusView();
}
function cancelPaste() {
  pasteDraft.value = undefined;
  focusView();
}
/** Keep the caret in the terminal so tapping a control never closes the keyboard. */
function keepFocus(event: Event) {
  event.preventDefault();
}

let gesture: { x: number; y: number; at: number } | undefined;
function gestureStart(event: PointerEvent) {
  if (event.button === 2) {
    rightClickSelection = view?.getSelection() || window.getSelection()?.toString() || "";
    return;
  }
  gesture = { x: event.clientX, y: event.clientY, at: Date.now() };
}
/**
 * A tap on the terminal: by default the keyboard comes up, since that is where
 * the person wants to type and a phone opens it only from inside a gesture;
 * with row selection armed, the tapped row is selected instead.
 *
 * Tap a row to move the selection to it.
 *
 * The distance from the cursor row to the tapped row is a number of arrow
 * presses, which holds because a list keeps the cursor on the row it has
 * highlighted -- verified against the client's own menu, where the cursor row
 * and the marked row are the same row.
 */
function gestureEnd(event: PointerEvent) {
  const start = gesture;
  gesture = undefined;
  if (!start || !view?.ready) return;
  const moved = Math.hypot(event.clientX - start.x, event.clientY - start.y);
  if (!isTap({ movedPx: moved, elapsedMs: Date.now() - start.at, hasSelection: view.hasSelection() })) return;
  if (!tapSelect.value) {
    if (event.pointerType !== "mouse") focusView();
    return;
  }
  const target = view.tappedRow(event.clientY);
  if (!target) return;
  const move = selectionPresses(target.cursor, target.row);
  if (move) sendKeys(repeatArrow(move.key, move.count, view.applicationCursorKeys()));
}

function updateScrolledBack() {
  scrolledBack.value = !!view?.isScrolledBack();
}
function scrollToLatest() {
  view?.scrollToBottom();
  updateScrolledBack();
}

function openSearch() {
  if (!view?.search) return;
  moreKeys.value = false;
  if (searchOpen.value) searchBar.value?.focus();
  else searchOpen.value = true;
}
function closeSearch() {
  searchOpen.value = false;
  searchProgress.value = undefined;
  view?.search?.clear();
  focusView();
}
function stepSearch(direction: 1 | -1) {
  if (direction > 0) view?.search?.next();
  else view?.search?.previous();
}
/** Taken before the terminal sees it, which would otherwise hand the key to the shell. */
function terminalKeydown(event: KeyboardEvent) {
  if (event.isComposing) return;
  const hasSelection = !!(view?.hasSelection() || window.getSelection()?.toString());
  const clipboard = terminalClipboardKey(event, undefined, hasSelection);
  if (clipboard) {
    event.preventDefault();
    event.stopPropagation();
    if (clipboard === "paste") {
      void pressPaste();
      return;
    }
    void copySelection(clearsSelectionOnCopy(event));
    return;
  }
  if (!view?.search || !isTerminalFind(event)) return;
  event.preventDefault();
  event.stopPropagation();
  openSearch();
}
watch(searchQuery, query => {
  if (!view?.search) return;
  if (query.trim()) view.search.find(query);
  else {
    view.search.clear();
    searchProgress.value = undefined;
  }
});

function connect() {
  if (disposed || !props.sessionId) return;
  try {
    createView();
    const sessionId = props.sessionId;
    // Asked again on every automatic retry, so each one resumes from what the
    // screen shows by then.
    stream.connect(() => streamUrl(window.location.href, sessionId, view?.size(), resume));
    resize();
  } catch {
    stream.fail(TERMINAL_VIEW_ERROR);
    cleanupView();
  }
}
defineExpose({ reconnect: connect });

function createView() {
  if (view) return;
  if (!host.value) throw new Error("Terminal host is unavailable");
  const create = engine === "wterm" ? createWtermView : createXtermView;
  view = create(host.value, terminalTheme(dark.value), viewEvents);
  observer = new ResizeObserver(resize);
  observer.observe(host.value);
  if (view.ready) viewEvents.ready();
}
watch(dark, isDark => view?.applyTheme(terminalTheme(isDark)));
// Larger text is fewer columns in the same width: tell the process.
watch(textScale, resize);

function cleanupView() {
  cancelAnimationFrame(resizeFrame);
  window.clearTimeout(replayTimer);
  replaying = false;
  terminalCwd = undefined;
  contextMenu.value = undefined;
  pendingOscCopy.value = undefined;
  zmodem.reset();
  scrolledBack.value = false;
  searchOpen.value = false;
  searchProgress.value = undefined;
  searchable.value = false;
  observer?.disconnect();
  observer = undefined;
  view?.dispose();
  view = undefined;
  host.value?.replaceChildren();
}
onMounted(() => {
  // `any-pointer: coarse` rather than `pointer: coarse`: a laptop with a
  // touchscreen keeps its keyboard, and showing touch-only controls there would
  // be clutter over the terminal.
  try {
    touchDevice.value = window.matchMedia?.("(any-pointer: coarse)").matches === true;
  } catch {
    touchDevice.value = false;
  }
  connect();
});
/**
 * Coming back to the page re-attaches a view that dropped meanwhile, and one
 * that may only look attached after a long absence. Only the view: an exited
 * process stays exited, and nothing typed is replayed.
 */
let stopPageReturn = () => {};
onMounted(() => {
  stopPageReturn = onPageReturn(stale => {
    // A view that never got a terminal starts over; any other is woken, which
    // retries a dropped socket, replaces a stale one and probes the rest.
    if (!view?.ready && state.value === "error") connect();
    else stream.wake(stale);
  });
});
watch(
  () => props.sessionId,
  async (_next, _previous, onCleanup) => {
    let current = true;
    onCleanup(() => {
      current = false;
    });
    stream.disconnect();
    zmodem.reset();
    terminalCwd = undefined;
    contextMenu.value = undefined;
    pendingOscCopy.value = undefined;
    resume = undefined;
    view?.reset();
    await nextTick();
    if (current) connect();
  },
);
onUnmounted(() => {
  files.dispose();
  stopPageReturn();
  disposed = true;
  stream.dispose();
  cleanupView();
  emit("status", false);
});
</script>

<template>
  <div
    :class="['native-terminal', { dark: dark, dragging }]"
    @keydown.capture="terminalKeydown"
    @paste.capture="files.onPaste"
    @dragover="files.onDragOver"
    @dragleave="files.onDragLeave"
    @drop="files.onDrop"
  >
    <div class="terminal-connection">
      <span :class="['state-dot', state]" /><span>{{ t(state === "connected" ? "Live · native client" : state) }}</span>
      <button
        v-if="state === 'disconnected' || state === 'error'"
        class="text-button"
        :aria-label="t('Reconnect session stream')"
        @click="connect"
      >
        <Icon name="refresh" :size="13" />{{ t("Reconnect") }}
      </button>
      <button
        v-if="state === 'connected'"
        type="button"
        class="text-button terminal-attach"
        :title="t('Add files or images: paste, drop, or pick them; their paths are typed into the terminal')"
        @click="pickFiles"
      >
        <Icon name="plus" :size="13" />{{ t("Add file") }}
      </button>
      <button
        v-if="searchable"
        type="button"
        class="text-button terminal-find"
        :title="t('Find in terminal') + ' · ' + findShortcut"
        @pointerdown="keepFocus"
        @click="openSearch"
      >
        <Icon name="search" :size="13" />{{ t("Find") }}
      </button>
    </div>
    <input
      ref="fileInput"
      class="sr-only"
      type="file"
      multiple
      tabindex="-1"
      aria-hidden="true"
      @change="filesPicked"
    />
    <input
      ref="rzFileInput"
      class="sr-only"
      type="file"
      multiple
      tabindex="-1"
      aria-hidden="true"
      @change="rzFilesPicked"
    />
    <div v-if="dragging" class="terminal-drop" aria-hidden="true">
      <span>{{ t("Drop to add the files and type their paths") }}</span>
    </div>
    <TerminalUploads v-if="uploads.length" :uploads="uploads" @retry="files.retry" @dismiss="files.dismiss" />
    <div
      v-if="pendingOscCopy && zmodemStatus.phase === 'idle'"
      class="terminal-uploads terminal-zmodem"
      role="status"
      aria-live="polite"
    >
      <div class="terminal-upload">
        <Icon name="clipboard" :size="16" />
        <span class="terminal-upload-text">
          <strong>{{ t("Terminal copied text") }}</strong>
          <small>{{ t("Tap Copy to place it on your clipboard") }}</small>
        </span>
        <button type="button" class="text-button" @click="flushPendingOscCopy">{{ t("Copy") }}</button>
        <button type="button" class="icon-button" :aria-label="t('Cancel')" @click="pendingOscCopy = undefined">
          <Icon name="close" :size="13" />
        </button>
      </div>
    </div>
    <div v-if="zmodemStatus.phase !== 'idle'" class="terminal-uploads terminal-zmodem" role="status" aria-live="polite">
      <div class="terminal-upload">
        <Icon :name="zmodemStatus.phase === 'downloading' ? 'download' : 'upload'" :size="16" />
        <span v-if="zmodemStatus.phase === 'awaiting_files'" class="terminal-upload-text">
          <strong>{{ t("ZMODEM upload (rz)") }}</strong>
          <small>{{ t("Choose files to send to the remote terminal") }}</small>
        </span>
        <span v-else-if="zmodemStatus.phase === 'downloading'" class="terminal-upload-text">
          <strong :title="zmodemStatus.name">{{ zmodemStatus.name }}</strong>
          <small>{{
            t("Downloading {progress}", {
              progress: formatTransferProgress(zmodemStatus.transferred, zmodemStatus.total),
            })
          }}</small>
        </span>
        <span v-else-if="zmodemStatus.phase === 'uploading'" class="terminal-upload-text">
          <strong :title="zmodemStatus.name">{{ zmodemStatus.name }}</strong>
          <small>{{
            t("Sending {progress}", {
              progress: formatTransferProgress(zmodemStatus.transferred, zmodemStatus.total),
            })
          }}</small>
        </span>
        <button v-if="zmodemStatus.phase === 'awaiting_files'" type="button" class="text-button" @click="pickRzFiles">
          {{ t("Choose files") }}
        </button>
        <button type="button" class="icon-button" :aria-label="t('Cancel')" @click="cancelZmodem">
          <Icon name="close" :size="13" />
        </button>
      </div>
    </div>
    <div
      v-if="notice"
      :class="state === 'error' ? 'inline-error' : 'inline-notice'"
      :role="state === 'error' ? 'alert' : 'status'"
    >
      {{ nativeNotice ? notice : t(notice) }}
    </div>
    <div
      ref="host"
      class="terminal-host"
      tabindex="-1"
      @focus.self="focusView"
      :aria-label="t('Native session {id}', { id: sessionId })"
      @contextmenu="openContextMenu"
      @pointerdown="gestureStart"
      @pointerup="gestureEnd"
      @pointercancel="gesture = undefined"
    />
    <ContextMenu
      v-if="contextMenu"
      :x="contextMenu.x"
      :y="contextMenu.y"
      :label="t('Terminal')"
      :width="210"
      @close="contextMenu = undefined"
    >
      <template v-if="contextMenu.link">
        <button type="button" @click="menuOpenLink">
          <Icon :name="contextMenu.link.kind === 'file' ? 'file' : 'globe'" :size="15" />
          <span>{{
            contextMenu.link.kind === "file" ? t("Open {path}", { path: contextMenu.link.text }) : t("Open link")
          }}</span>
        </button>
        <hr />
      </template>
      <button type="button" :disabled="!contextMenu.selectionText" @click="menuCopy">
        <Icon name="clipboard" :size="15" /><span>{{ t("Copy") }}</span>
      </button>
      <button type="button" @click="menuPaste">
        <Icon name="plus" :size="15" /><span>{{ t("Paste") }}</span>
      </button>
      <button type="button" @click="menuSelectAll">
        <Icon name="grid" :size="15" /><span>{{ t("Select all") }}</span>
      </button>
      <template v-if="searchable">
        <hr />
        <button type="button" @click="menuFind">
          <Icon name="search" :size="15" /><span>{{ t("Find") }}</span>
        </button>
      </template>
      <template v-if="contextMenu.mouseTracking && !contextMenu.selectionText">
        <hr />
        <p class="context-menu-note">{{ t("Hold Shift while dragging to select text") }}</p>
      </template>
    </ContextMenu>
    <TerminalSearchBar
      v-if="searchOpen"
      ref="searchBar"
      v-model:query="searchQuery"
      :count="searchCount"
      :can-step="!!searchProgress?.count"
      @step="stepSearch"
      @close="closeSearch"
    />
    <button
      v-if="scrolledBack"
      type="button"
      :class="['terminal-latest', { 'above-keys': touchDevice, 'above-more': touchDevice && moreKeys }]"
      :aria-label="t('Jump to latest output')"
      :title="t('Jump to latest output')"
      @pointerdown="keepFocus"
      @click="scrollToLatest"
    >
      <Icon name="chevron" :size="14" /><span>{{ t("Latest") }}</span>
    </button>
    <TerminalKeyBar
      v-if="touchDevice"
      :client="keyClient(provider)"
      v-model:ctrl-armed="ctrlArmed"
      v-model:shift-armed="shiftArmed"
      v-model:tap-select="tapSelect"
      v-model:more-keys="moreKeys"
      @keys="sendPresses"
      @tab="sendKeys(tabSequence({ shift: shiftArmed }))"
      @arrow="pressArrow"
      @enter="sendKeys(ENTER_SEQUENCE)"
      @paste="pressPaste"
      @pick-files="pickFiles"
    />
    <form
      v-if="pasteDraft !== undefined"
      class="terminal-paste"
      @submit.prevent="sendPaste"
      @keydown.esc.prevent="cancelPaste"
    >
      <textarea
        ref="pasteField"
        v-model="pasteDraft"
        :aria-label="t('Text to paste')"
        :placeholder="t('Long-press here to paste text, a photo or a file, then send it to the terminal')"
        @paste="onFieldPaste"
      />
      <div>
        <button type="button" class="secondary-button" @click="cancelPaste">{{ t("Cancel") }}</button><button
          type="submit"
          class="primary-button"
          :disabled="!pasteDraft"
        >
          {{ t("Send") }}
        </button>
      </div>
    </form>
  </div>
</template>
