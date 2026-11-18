import { computed, nextTick, onBeforeUnmount, ref, watch, type Ref } from "vue";
import type { Virtualizer } from "@tanstack/vue-virtual";
import type { DisplayItem } from "./tool-runs";
import type { ToolItem } from "./chat-model";
import { request, sessionPath } from "./api";

type Translate = (text: string, values?: Record<string, string | number>) => string;

/** One place a search found, as the server reports it. */
export interface SearchHit {
  seq: number;
  kind: "message" | "tool";
  id: string;
  snippet: string;
}
export interface SearchResult {
  hits: SearchHit[];
  more: boolean;
}

export function parseSearchResult(value: unknown): SearchResult {
  if (!value || typeof value !== "object") throw Error("Invalid search response.");
  const v = value as Record<string, unknown>;
  if (!Array.isArray(v.hits) || typeof v.more !== "boolean") throw Error("Invalid search response.");
  const hits = v.hits.map(hit => {
    const h = hit as Record<string, unknown>;
    if (
      !h ||
      typeof h !== "object" ||
      typeof h.seq !== "number" ||
      !Number.isSafeInteger(h.seq) ||
      (h.kind !== "message" && h.kind !== "tool") ||
      typeof h.id !== "string" ||
      typeof h.snippet !== "string"
    )
      throw Error("Invalid search response.");
    return { seq: h.seq, kind: h.kind, id: h.id, snippet: h.snippet } as SearchHit;
  });
  return { hits, more: v.more };
}

/**
 * The row a hit is shown in: a message is its own row; a call sits in a run
 * of calls, possibly nested inside a subagent's. `path` names the calls to
 * open, outermost first, for one shown folded inside others.
 */
export function rowForHit(rows: readonly DisplayItem[], hit: SearchHit): { index: number; path: string[] } | undefined {
  for (let index = 0; index < rows.length; index++) {
    const row = rows[index]!;
    if (hit.kind === "message" && row.type === "message" && row.id === hit.id) return { index, path: [] };
    if (hit.kind === "tool" && row.type === "tool_run") {
      for (const tool of row.tools) {
        const path = pathTo(tool, hit.id);
        if (path) return { index, path };
      }
    }
  }
  return undefined;
}
function pathTo(tool: ToolItem, id: string): string[] | undefined {
  if (tool.id === id) return [tool.id];
  for (const child of tool.children ?? []) {
    const path = pathTo(child, id);
    if (path) return [tool.id, ...path];
  }
  return undefined;
}

/** The next hit to show, stepping through them in either direction and around. */
export function stepHit(count: number, current: number, direction: 1 | -1): number {
  if (!count) return -1;
  if (current < 0) return direction === 1 ? 0 : count - 1;
  return (current + direction + count) % count;
}

/** Where `query` appears in `text`, case aside for Latin letters, as [start, end) offsets. */
export function occurrences(text: string, query: string): Array<[number, number]> {
  const needle = query.trim().toLowerCase();
  if (!needle) return [];
  const haystack = text.toLowerCase(),
    found: Array<[number, number]> = [];
  // Lower-casing keeps lengths for the scripts searched here; offsets still line up.
  if (haystack.length !== text.length) return found;
  for (let at = haystack.indexOf(needle); at !== -1; at = haystack.indexOf(needle, at + needle.length))
    found.push([at, at + needle.length]);
  return found;
}

/** What the search box says beside the query: working, nothing, or which match of how many. */
export function searchCount(
  state: { busy: boolean; query: string; hits: number; index: number; more: boolean },
  t: Translate,
): string {
  if (state.busy) return "…";
  if (!state.query.trim()) return "";
  if (!state.hits) return t("No matches");
  return `${state.index + 1} / ${state.hits}${state.more ? "+" : ""}`;
}

export interface ConversationSearchOptions {
  sessionId: () => string;
  viewport: Ref<HTMLElement | undefined>;
  rows: () => readonly DisplayItem[];
  windowed: () => Virtualizer<HTMLElement, Element> | undefined;
  /** Whether the view holds the event `seq`, and how to read the page before it. */
  holds: (seq: number) => boolean;
  loadOlder: () => Promise<void>;
  /** A match brought into view stops the view following new output. */
  stopFollowing: () => void;
  t: Translate;
}

/**
 * Search through the whole conversation (the server reads its full history):
 * each match is brought into view -- older pages read as far as it needs, the
 * calls it is folded inside opened -- and the words are highlighted where they
 * show, with the CSS highlight registry, so no row is changed to mark them.
 */
export function useConversationSearch(options: ConversationSearchOptions) {
  const { viewport, windowed, t } = options;
  const searchOpen = ref(false),
    searchQuery = ref(""),
    searchHits = ref<SearchHit[]>([]),
    searchMore = ref(false),
    searchIndex = ref(-1),
    searchBusy = ref(false),
    searchNotice = ref(""),
    searchInput = ref<HTMLInputElement>();
  let searchTimer: ReturnType<typeof setTimeout> | undefined,
    searchGeneration = 0,
    searchRow = -1;
  const count = computed(() =>
    searchCount(
      {
        busy: searchBusy.value,
        query: searchQuery.value,
        hits: searchHits.value.length,
        index: searchIndex.value,
        more: searchMore.value,
      },
      t,
    ),
  );
  function openSearch() {
    searchOpen.value = true;
    void nextTick(() => {
      searchInput.value?.focus();
      searchInput.value?.select();
    });
  }
  function closeSearch() {
    clearTimeout(searchTimer);
    searchGeneration++;
    searchOpen.value = false;
    searchHits.value = [];
    searchIndex.value = -1;
    searchRow = -1;
    searchBusy.value = false;
    searchNotice.value = "";
    paintHighlights();
  }
  watch(searchQuery, query => {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => void runSearch(query), 250);
  });
  watch(options.sessionId, () => {
    if (searchOpen.value) closeSearch();
  });
  async function runSearch(query: string) {
    const own = ++searchGeneration,
      id = options.sessionId();
    searchNotice.value = "";
    searchRow = -1;
    if (!query.trim()) {
      searchHits.value = [];
      searchIndex.value = -1;
      searchBusy.value = false;
      paintHighlights();
      return;
    }
    searchBusy.value = true;
    try {
      const result = parseSearchResult(
        await request<unknown>(`${sessionPath(id)}/conversation/search?q=${encodeURIComponent(query.trim())}`),
      );
      if (own !== searchGeneration || options.sessionId() !== id) return;
      searchHits.value = result.hits;
      searchMore.value = result.more;
      searchIndex.value = -1;
      searchBusy.value = false;
      if (result.hits.length) await showHit(0);
      else paintHighlights();
    } catch {
      if (own === searchGeneration) {
        searchBusy.value = false;
        searchNotice.value = "Search failed.";
      }
    }
  }
  async function stepSearch(direction: 1 | -1) {
    const next = stepHit(searchHits.value.length, searchIndex.value, direction);
    if (next >= 0) await showHit(next);
  }
  async function showHit(index: number) {
    const hit = searchHits.value[index],
      own = searchGeneration;
    if (!hit) return;
    searchIndex.value = index;
    searchNotice.value = "";
    for (let pages = 0; pages < 500 && !options.holds(hit.seq); pages++) {
      await options.loadOlder();
      if (own !== searchGeneration) return;
    }
    const target = rowForHit(options.rows(), hit);
    if (!target) {
      searchRow = -1;
      searchNotice.value = "This match is not shown in the conversation.";
      paintHighlights();
      return;
    }
    options.stopFollowing();
    searchRow = target.index;
    windowed()?.scrollToIndex(target.index, { align: "center" });
    await nextTick();
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    if (own !== searchGeneration) return;
    const row = viewport.value?.querySelector(`.chat-row[data-index="${target.index}"]`);
    if (row) {
      // A run of calls, and each call the match is folded inside, open to show it.
      if (row.firstElementChild instanceof HTMLDetailsElement) row.firstElementChild.open = true;
      for (const id of target.path) {
        const fold = row.querySelector(`[data-tool-id="${CSS.escape(id)}"]`);
        if (fold instanceof HTMLDetailsElement) fold.open = true;
      }
      windowed()?.scrollToIndex(target.index, { align: "center" });
    }
    // Let the row settle where the virtualizer put it (it keeps steering to the
    // row for a few frames), then bring the words themselves to the middle: a
    // long reply is taller than the view. Through the virtualizer, so the row
    // it was steering to does not pull the view back.
    for (let frame = 0; frame < 4; frame++) await new Promise(resolve => requestAnimationFrame(resolve));
    if (own !== searchGeneration) return;
    const words = paintHighlights(),
      el = viewport.value;
    if (words && el) {
      const at = words.getBoundingClientRect(),
        view = el.getBoundingClientRect();
      if (at.height) windowed()?.scrollBy(at.top - view.top - (view.height - at.height) / 2);
    }
  }
  /** Marks the matches in the rows shown; the first in the current match's row stands out, and is returned. */
  function paintHighlights(): Range | undefined {
    const registry = (globalThis.CSS as unknown as { highlights?: Map<string, unknown> } | undefined)?.highlights;
    const HighlightType = (globalThis as unknown as { Highlight?: new (...ranges: Range[]) => unknown }).Highlight;
    if (!registry || !HighlightType) return undefined;
    registry.delete("chat-search");
    registry.delete("chat-search-current");
    const query = searchOpen.value ? searchQuery.value : "",
      el = viewport.value;
    if (!query.trim() || !el) return undefined;
    const all: Range[] = [],
      current: Range[] = [];
    for (const row of el.querySelectorAll<HTMLElement>(".chat-row")) {
      const inCurrent = Number(row.dataset.index) === searchRow;
      const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        for (const [start, end] of occurrences(node.textContent ?? "", query)) {
          const range = new Range();
          range.setStart(node, start);
          range.setEnd(node, end);
          (inCurrent && !current.length ? current : all).push(range);
        }
      }
    }
    registry.set("chat-search", new HighlightType(...all));
    if (current.length) registry.set("chat-search-current", new HighlightType(...current));
    return current[0];
  }
  // Rows coming into view while searching are marked too.
  function onRowsRange() {
    if (searchOpen.value && searchQuery.value.trim()) void nextTick(paintHighlights);
  }
  onBeforeUnmount(() => {
    clearTimeout(searchTimer);
    if (searchOpen.value) {
      searchOpen.value = false;
      paintHighlights();
    }
  });
  function searchKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeSearch();
      return;
    }
    if (event.key === "Enter" && !event.isComposing) {
      event.preventDefault();
      void stepSearch(event.shiftKey ? -1 : 1);
    }
  }
  return {
    searchOpen,
    searchQuery,
    searchHits,
    searchNotice,
    searchInput,
    searchCount: count,
    openSearch,
    closeSearch,
    stepSearch,
    onRowsRange,
    searchKeydown,
  };
}
