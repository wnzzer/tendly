import { nextTick, onBeforeUnmount, ref, watch, type Ref } from "vue";
import type { Virtualizer } from "@tanstack/vue-virtual";
import { request, sessionPath } from "./api";
import { parseConversationPage, prependChatEvents, type ChatEvent } from "./chat-model";

/**
 * The history before what the view holds. A conversation opens on its latest
 * page; the pages before it are read as the view nears the top, and put in
 * front without moving what is being read.
 */

/** How close to the top, in pixels, the page before is read: it is there by the time it is scrolled to. */
export const OLDER_MARGIN = 1200;

/** The first row in view and how far it sits below the top, to keep it there when rows are put in front of it. */
export function anchorRow(
  items: readonly { key: unknown; start: number; end: number }[],
  scrollTop: number,
): { key: string; offset: number } | undefined {
  const anchor = items.find(item => item.end > scrollTop);
  return anchor ? { key: String(anchor.key), offset: anchor.start - scrollTop } : undefined;
}

export interface OlderHistoryOptions {
  sessionId: () => string;
  preview: () => boolean;
  mounted: () => boolean;
  /** Which read of the conversation the view is on; a page for an earlier one is dropped. */
  generation: () => number;
  events: Ref<ChatEvent[]>;
  viewport: Ref<HTMLElement | undefined>;
  windowed: () => Virtualizer<HTMLElement, Element> | undefined;
  rowKeys: () => readonly string[];
  /** Called once rows were put in front, as what sits above the rows may have changed. */
  measured: () => void;
  failed: (message: string) => void;
}

export function useOlderHistory(options: OlderHistoryOptions) {
  const { events, viewport, windowed } = options;
  /** The cursor to the history before what the view holds, while there is any. */
  const older = ref<number | undefined>(),
    loadingOlder = ref(false),
    olderSentinel = ref<HTMLElement>();
  /** Whether the view holds the event `seq`: everything from the older cursor on is held. */
  const holds = (seq: number) => older.value === undefined || seq >= older.value;
  /** Reads the page before what the view holds, once at a time for everyone who asks. */
  let olderRead: Promise<void> | undefined;
  function loadOlder(): Promise<void> {
    olderRead ??= readOlder().finally(() => {
      olderRead = undefined;
    });
    return olderRead;
  }
  /** The page before what the view holds, keeping the row in view where it was. */
  async function readOlder() {
    const id = options.sessionId(),
      own = options.generation(),
      before = older.value,
      el = viewport.value;
    if (before === undefined || options.preview() || !el) return;
    const current = () => own === options.generation();
    loadingOlder.value = true;
    try {
      const page = parseConversationPage(await request<unknown>(`${sessionPath(id)}/conversation?before=${before}`));
      if (!current() || options.sessionId() !== id || !options.mounted()) return;
      const anchor = anchorRow(windowed()?.getVirtualItems() ?? [], el.scrollTop);
      events.value = prependChatEvents(page.events, events.value);
      older.value = page.older;
      await nextTick();
      const index = anchor ? options.rowKeys().indexOf(anchor.key) : -1;
      if (anchor && index >= 0) {
        const start = windowed()?.getOffsetForIndex(index, "start")?.[0];
        if (start !== undefined) el.scrollTop = start - anchor.offset;
      }
      options.measured();
    } catch {
      if (current()) options.failed("Could not load earlier messages.");
    } finally {
      if (current()) loadingOlder.value = false;
    }
    // A short page may leave the top still in reach.
    if (current() && older.value !== undefined && el.scrollTop < OLDER_MARGIN) queueMicrotask(() => void loadOlder());
  }
  let olderObserver: IntersectionObserver | undefined;
  watch(olderSentinel, sentinel => {
    olderObserver?.disconnect();
    olderObserver = undefined;
    if (!sentinel || !viewport.value || typeof IntersectionObserver !== "function") return;
    olderObserver = new IntersectionObserver(
      entries => {
        if (entries.some(entry => entry.isIntersecting)) void loadOlder();
      },
      { root: viewport.value, rootMargin: `${OLDER_MARGIN}px 0px 0px 0px` },
    );
    olderObserver.observe(sentinel);
  });
  onBeforeUnmount(() => olderObserver?.disconnect());
  return { older, loadingOlder, olderSentinel, holds, loadOlder };
}
