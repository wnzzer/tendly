/**
 * Right-click in Tendly's own chrome -- headers, the sidebar, the status
 * bar, buttons, empty canvas -- belongs to Tendly, not to the browser's
 * page menu ("Back", "Reload", "Save as…"), which only ever does something
 * wrong there. Places with a menu of their own open it and stop here.
 *
 * The browser's menu stays wherever it does real work: text being typed or
 * selected, links and images, and the terminal, which draws its own.
 */
const NATIVE_TARGETS =
  'input, textarea, select, [contenteditable=""], [contenteditable="true"], a[href], img, video, .xterm, .wterm, .terminal-host, .chat-markdown';

export function keepsNativeMenu(target: EventTarget | null, selection = "") {
  if (selection.trim()) return true;
  // Text nodes have no closest(); ask their parent element.
  const node = target as (Partial<Element> & { parentElement?: Element | null }) | null;
  const element = typeof node?.closest === "function" ? (node as Element) : (node?.parentElement ?? null);
  return !element || !!element.closest(NATIVE_TARGETS);
}

export function installContextMenuPolicy(doc: Document = document) {
  // Bubble phase, last: a component that opened its own menu has already
  // called preventDefault, and one that chose to leave it alone is respected.
  const listener = (event: MouseEvent) => {
    if (event.defaultPrevented || keepsNativeMenu(event.target, String(doc.getSelection?.() ?? ""))) return;
    event.preventDefault();
  };
  doc.addEventListener("contextmenu", listener);
  return () => doc.removeEventListener("contextmenu", listener);
}
