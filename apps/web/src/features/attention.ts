import { computed, reactive, ref } from "vue";
import type { Session } from "@tendly/protocol";
import { showToast } from "./toasts";

/**
 * Which sessions are waiting on you -- an approval, a question -- as opposed to
 * merely running, and saying so where you will see it.
 *
 * The statuses live here, outside any one view, so a tab, a sidebar row and
 * the page title all read the same answer without it being passed down to
 * them. A session that starts waiting while you look elsewhere gets a toast;
 * while the page is in the background, a desktop notification if you asked
 * for them.
 */
export const sessionStatuses = reactive<Record<string, Session["status"]>>({});
export const waitingIds = computed(() => Object.keys(sessionStatuses).filter(id => sessionStatuses[id] === "waiting"));

const KEY = "tendly.notifications";
function stored(): boolean {
  try {
    return localStorage.getItem(KEY) === "on";
  } catch {
    return false;
  }
}
export const desktopNotifications = ref(stored());

export type NotificationSupport = "unsupported" | "insecure" | "denied" | "default" | "granted";
/** Browsers offer notifications only to a secure page: https, or localhost. */
export function notificationSupport(): NotificationSupport {
  if (typeof window === "undefined" || !("Notification" in window)) return "unsupported";
  if (!window.isSecureContext) return "insecure";
  return Notification.permission;
}

/** Turning them on asks the browser, from the click that asked for it. */
export async function setDesktopNotifications(on: boolean): Promise<boolean> {
  let enabled = on;
  if (on) {
    const support = notificationSupport();
    if (support === "unsupported" || support === "insecure") enabled = false;
    else if (support !== "granted") enabled = (await Notification.requestPermission()) === "granted";
  }
  desktopNotifications.value = enabled;
  try {
    if (enabled) localStorage.setItem(KEY, "on");
    else localStorage.removeItem(KEY);
  } catch {
    /* This page only. */
  }
  return enabled;
}

export interface AttentionContext {
  /** The session in the focused pane: already in front of you, so no toast. */
  focusedSessionId?: string;
  open(session: Session): void;
  t(key: string, values?: Record<string, string | number>): string;
}

/** Record a fresh session list, and announce each session that has just begun waiting. */
export function trackSessions(list: readonly Session[], context: AttentionContext) {
  const started = list.filter(session => {
    const before = sessionStatuses[session.id];
    return before !== undefined && before !== "waiting" && session.status === "waiting";
  });
  const present = new Set(list.map(session => session.id));
  for (const id of Object.keys(sessionStatuses)) if (!present.has(id)) delete sessionStatuses[id];
  for (const session of list) sessionStatuses[session.id] = session.status;
  for (const session of started) announce(session, context);
}

function announce(session: Session, context: AttentionContext) {
  const away = typeof document !== "undefined" && (document.hidden || !document.hasFocus());
  const title = context.t("{session} is waiting for you", { session: session.title });
  if (away) {
    if (!desktopNotifications.value || notificationSupport() !== "granted") return;
    const notification = new Notification(title, {
      body: context.t("An approval or a question needs your answer."),
      tag: `tendly-${session.id}`,
    });
    notification.onclick = () => {
      window.focus();
      context.open(session);
      notification.close();
    };
    return;
  }
  if (session.id === context.focusedSessionId) return;
  showToast(title, { tone: "warn", action: { label: context.t("View"), run: () => context.open(session) } });
}

/** The page title, with how many sessions are waiting in front of it. */
export function attentionTitle(base: string, waiting: number): string {
  return waiting > 0 ? `(${waiting}) ${base}` : base;
}
