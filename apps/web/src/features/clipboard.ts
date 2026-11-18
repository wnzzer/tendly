/** Let phones keep their native long-press selection/copy menu. */
export function preserveNativeContextMenu(event: MouseEvent, mobile: boolean): boolean {
  return (
    mobile ||
    ("pointerType" in event && event.pointerType === "touch") ||
    (typeof window !== "undefined" && window.matchMedia?.("(any-pointer: coarse)").matches === true)
  );
}

/** Call from the click itself so mobile browsers retain user activation. */
export async function copyText(text: string): Promise<void> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return;
    }
  } catch {
    /* HTTP LAN pages and denied permissions can use native copy. */
  }
  const active = document.activeElement as HTMLElement | null;
  const input = active instanceof HTMLTextAreaElement || active instanceof HTMLInputElement ? active : null;
  const start = input?.selectionStart,
    end = input?.selectionEnd;
  const selection = window.getSelection();
  const ranges = selection
    ? Array.from({ length: selection.rangeCount }, (_, i) => selection.getRangeAt(i).cloneRange())
    : [];
  const field = document.createElement("textarea");
  field.value = text;
  field.readOnly = true;
  field.style.cssText = "position:fixed;top:0;left:0;width:1px;height:1px;opacity:0;font-size:16px";
  document.body.append(field);
  try {
    field.focus({ preventScroll: true });
    field.select();
    field.setSelectionRange(0, text.length);
    if (!document.execCommand("copy")) throw new Error("Clipboard unavailable");
  } finally {
    field.remove();
    active?.focus?.({ preventScroll: true });
    if (input && start != null && end != null) input.setSelectionRange(start, end);
    if (selection && ranges.length) {
      selection.removeAllRanges();
      for (const range of ranges) selection.addRange(range);
    }
  }
}
