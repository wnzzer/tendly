/**
 * Arrow keys in the groups that are one choice among several: tab strips
 * (role=tablist), option groups built of buttons (role=radiogroup) and the
 * vertical navigation lists marked data-arrow-nav. The arrows move along the
 * group and choose as they go, Home and End jump to its ends -- the same
 * keys a native radio group or the tree already answer. A widget that
 * handles its own arrows prevents the default first and is left alone.
 */
const GROUPS: readonly { group: string; item: string; vertical: boolean }[] = [
  { group: '[role="tablist"]', item: '[role="tab"]', vertical: false },
  { group: '[role="radiogroup"]', item: 'button[role="radio"]', vertical: false },
  { group: '[data-arrow-nav]', item: 'button', vertical: true },
];

export function rovingTarget(event: Pick<KeyboardEvent, "key" | "altKey" | "ctrlKey" | "metaKey" | "shiftKey" | "target" | "defaultPrevented">): HTMLElement | undefined {
  if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return undefined;
  const target = event.target instanceof Element ? event.target : null;
  for (const { group, item, vertical } of GROUPS) {
    const owner = target?.closest<HTMLElement>(group);
    const own = target?.closest<HTMLElement>(item);
    if (!owner || !own || !owner.contains(own)) continue;
    const items = Array.from(owner.querySelectorAll<HTMLElement>(item)).filter(element => !(element as HTMLButtonElement).disabled && element.closest(group) === owner);
    const index = items.indexOf(own);
    const back = vertical ? "ArrowUp" : "ArrowLeft", forth = vertical ? "ArrowDown" : "ArrowRight";
    const next = event.key === back ? index - 1 : event.key === forth ? index + 1 : event.key === "Home" ? 0 : event.key === "End" ? items.length - 1 : undefined;
    if (next === undefined || index < 0) return undefined;
    return items[(next + items.length) % items.length];
  }
  return undefined;
}

export function installRovingKeys(doc: Document = document) {
  const onKey = (event: KeyboardEvent) => {
    const next = rovingTarget(event);
    if (!next) return;
    event.preventDefault();
    next.focus();
    next.click();
  };
  doc.addEventListener('keydown', onKey);
  return () => doc.removeEventListener('keydown', onKey);
}
