import { nextTick, onBeforeUnmount, onMounted, ref, watch, type Ref } from "vue";

interface MenuRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
}
export function sessionMenuPosition(
  anchor: MenuRect,
  viewport: { width: number; height: number },
  panel: { width: number; height: number },
) {
  const inset = 8;
  const maxWidth = Math.max(0, viewport.width - inset * 2);
  const width = Math.min(panel.width, maxWidth);
  const left = Math.max(inset, Math.min(anchor.right - width, viewport.width - width - inset));
  const below = anchor.bottom + 5;
  const top =
    below + panel.height <= viewport.height - inset || anchor.top < panel.height + inset
      ? below
      : Math.max(inset, anchor.top - panel.height - 5);
  return { left, top, maxWidth, maxHeight: Math.max(0, viewport.height - top - inset) };
}

/** A style is written only when it says something new: each write re-renders
 * the pane that binds it, and the pane holds the whole conversation. */
export function sameStyle(a: Record<string, string>, b: Record<string, string>) {
  const keys = Object.keys(a);
  return keys.length === Object.keys(b).length && keys.every(key => a[key] === b[key]);
}

/** Whether a scroll could have moved the tab: only one of its own containers can. */
export function scrollMovesAnchor(target: EventTarget | null, anchor: Element | null) {
  if (
    !anchor ||
    !(target instanceof Node) ||
    target === document ||
    target === document.documentElement ||
    target === document.body
  )
    return true;
  return target.contains(anchor);
}

/** Follow the actual tab through splits, sidebar changes and scrolling. The
 * panel stays in the viewport even when the tab strip clips its neighbours.
 * A conversation scrolling, or its rows coming and going as it does, cannot
 * move the tab, so neither is followed: measuring the tab forces a layout,
 * which every scrolled frame would otherwise pay for. */
export function useSessionMenuPosition(paneId: () => string | undefined, menu: Ref<HTMLDetailsElement | undefined>) {
  const tabStyle = ref<Record<string, string>>(paneId() ? { visibility: "hidden" } : {});
  const panelStyle = ref<Record<string, string>>({});
  let anchor: HTMLElement | null = null;
  let observer: ResizeObserver | undefined, mutations: MutationObserver | undefined;
  let frame: number | undefined,
    mounted = false;
  const setTab = (style: Record<string, string>) => {
    if (!sameStyle(tabStyle.value, style)) tabStyle.value = style;
  };
  const setPanel = (style: Record<string, string>) => {
    if (!sameStyle(panelStyle.value, style)) panelStyle.value = style;
  };

  function update() {
    frame = undefined;
    if (!paneId()) {
      setTab({});
      setPanel({});
      return;
    }
    const target = document.getElementById(`session-actions-${paneId()}`);
    if (anchor !== target) {
      observer?.disconnect();
      mutations?.disconnect();
      anchor = target;
      for (let element: HTMLElement | null = anchor; element; element = element.parentElement)
        observer?.observe(element);
      if (anchor?.parentElement?.parentElement)
        mutations?.observe(anchor.parentElement.parentElement, { childList: true, subtree: true, characterData: true });
    }
    const rect = anchor?.getBoundingClientRect();
    const strip = anchor?.closest(".dock-tabs")?.getBoundingClientRect();
    if (!rect?.width || !rect.height || (strip && (rect.left < strip.left || rect.right > strip.right))) {
      setTab({ visibility: "hidden" });
      if (menu.value) menu.value.open = false;
      return;
    }
    const summary = menu.value?.querySelector("summary");
    const height = summary?.getBoundingClientRect().height || 28;
    setTab({
      position: "fixed",
      left: `${rect.left}px`,
      top: `${rect.top + (rect.height - height) / 2}px`,
      visibility: "visible",
    });
    const panel = menu.value?.querySelector<HTMLElement>("nav, .session-menu-content");
    if (menu.value?.open && panel) {
      const position = sessionMenuPosition(
        rect,
        { width: window.innerWidth, height: window.innerHeight },
        { width: panel.offsetWidth || 205, height: panel.scrollHeight || 260 },
      );
      setPanel({
        position: "fixed",
        left: `${position.left}px`,
        top: `${position.top}px`,
        right: "auto",
        maxWidth: `${position.maxWidth}px`,
        maxHeight: `${position.maxHeight}px`,
        overflowY: "auto",
      });
    }
  }
  function schedule() {
    if (mounted && frame === undefined) frame = requestAnimationFrame(update);
  }
  function scrolled(event: Event) {
    if (scrollMovesAnchor(event.target, anchor)) schedule();
  }
  /** A row of a conversation mounted or unmounted as it scrolls leaves the tab where it was. */
  function mutated(records: MutationRecord[]) {
    if (
      records.some(record => {
        const node = record.target instanceof Element ? record.target : record.target.parentElement;
        return !node?.closest(".chat-timeline");
      })
    )
      schedule();
  }
  function outside(event: PointerEvent) {
    if (menu.value?.open && event.target instanceof Node && !menu.value.contains(event.target)) menu.value.open = false;
  }
  watch(paneId, async () => {
    await nextTick();
    schedule();
  });
  watch(menu, schedule);
  onMounted(() => {
    mounted = true;
    observer = new ResizeObserver(schedule);
    mutations = new MutationObserver(mutated);
    update();
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", scrolled, true);
    document.addEventListener("pointerdown", outside);
  });
  onBeforeUnmount(() => {
    mounted = false;
    observer?.disconnect();
    mutations?.disconnect();
    if (frame !== undefined) cancelAnimationFrame(frame);
    window.removeEventListener("resize", schedule);
    window.removeEventListener("scroll", scrolled, true);
    document.removeEventListener("pointerdown", outside);
  });
  return { tabStyle, panelStyle, positionMenu: schedule };
}
