import { reactive } from "vue";

/**
 * Short messages that confirm something happened, and go away by themselves.
 *
 * One place for them, instead of a banner per view that stayed until dismissed.
 * A toast may carry one action -- Undo, View -- and pauses while the pointer
 * or focus is on it, so it never disappears from under a click.
 */
export interface ToastAction {
  label: string;
  run: () => void;
}
export interface Toast {
  id: number;
  message: string;
  tone: "info" | "success" | "warn" | "error";
  action?: ToastAction;
  duration: number;
}

export const toasts = reactive<Toast[]>([]);
let next = 0;
const timers = new Map<number, ReturnType<typeof setTimeout>>();
const LIMIT = 3;

export function showToast(
  message: string,
  options: { tone?: Toast["tone"]; action?: ToastAction; duration?: number } = {},
): number {
  const toast: Toast = {
    id: ++next,
    message,
    tone: options.tone ?? "success",
    action: options.action,
    duration: options.duration ?? (options.action ? 8000 : 5000),
  };
  toasts.push(toast);
  while (toasts.length > LIMIT) dismissToast(toasts[0].id);
  resumeToast(toast.id);
  return toast.id;
}

export function dismissToast(id: number) {
  clearTimeout(timers.get(id));
  timers.delete(id);
  const index = toasts.findIndex(toast => toast.id === id);
  if (index >= 0) toasts.splice(index, 1);
}

export function pauseToast(id: number) {
  clearTimeout(timers.get(id));
  timers.delete(id);
}

export function resumeToast(id: number) {
  const toast = toasts.find(item => item.id === id);
  if (!toast || typeof setTimeout === "undefined") return;
  clearTimeout(timers.get(id));
  timers.set(
    id,
    setTimeout(() => dismissToast(id), toast.duration),
  );
}
