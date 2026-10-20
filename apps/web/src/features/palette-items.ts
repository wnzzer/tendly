import type { ProviderKind, Session, Workspace } from "@tendly/protocol";
import type { PaletteItem } from "./CommandPalette.vue";
import { PROVIDER_KINDS, providerLabel } from "./clients";
import { isSessionArchived } from "./session-list";
import { PREFIX_COMMANDS, formatChord, formatPrefix, platformShortcuts } from "./shortcuts";
import type { SettingsSection } from "../router";

/** The chord shown beside a command, when it has one. */
/** The chord for `id` on this platform, else its key after Ctrl+B. */
export function shortcutHint(id: string): string | undefined {
  const shortcut = platformShortcuts().find(item => item.id === id);
  if (shortcut) return formatChord(shortcut.chord);
  const command = PREFIX_COMMANDS.find(item => item.id === id);
  return command ? formatPrefix(command) : undefined;
}

/** What a palette entry does once chosen; the shell supplies each. */
export interface PaletteActions {
  openSession(session: Session): void;
  quickSession(workspaceId: string, provider: ProviderKind): void;
  newSession(workspaceId: string): void;
  openPage(name: "sessions" | "system" | "usage"): void;
  openCanvas(): void;
  openSettings(section: SettingsSection): void;
  openWorkspace(id: string): void;
  addWorkspace(): void;
  toggleSidebar(): void;
  toggleFiles(): void;
  setTheme(theme: "light" | "dark" | "system"): void;
  showShortcuts(): void;
}

/** How many of the most recently updated sessions are offered before anything is typed. */
const RECENT = 6;

/**
 * Everything the command palette can find: every session not archived, those
 * waiting on you first and the latest few suggested; new sessions in the
 * workspace in view; the pages, settings sections and workspaces; and the
 * shell's own commands.
 */
export function paletteItems(input: { sessions: readonly Session[]; workspaces: readonly Workspace[]; here: Workspace | undefined; t: (key: string, values?: Record<string, string>) => string }, actions: PaletteActions): PaletteItem[] {
  const { sessions, workspaces, here, t } = input, hint = shortcutHint;
  const workspaceName = (id: string) => workspaces.find(workspace => workspace.id === id)?.name ?? "";
  const label = (provider: ProviderKind) => provider === "terminal" ? t("Terminal") : providerLabel(provider);
  const listed = sessions.filter(session => !isSessionArchived(session));
  const recent = [...listed].sort((a, b) => Date.parse(b.updated_at ?? "") - Date.parse(a.updated_at ?? "")).slice(0, RECENT).map(session => session.id);
  const items: PaletteItem[] = [];
  for (const session of listed) {
    const waiting = session.status === "waiting";
    items.push({ id: `session:${session.id}`, title: session.title, subtitle: `${label(session.provider)} · ${workspaceName(session.workspace_id)}${waiting ? " · " + t("Needs you") : ""}`, group: waiting ? t("Needs you") : t("Sessions"), provider: session.provider, suggested: waiting || recent.includes(session.id), run: () => actions.openSession(session) });
  }
  if (here) {
    for (const provider of PROVIDER_KINDS) items.push({ id: `new:${provider}`, title: provider === "terminal" ? t("New terminal") : t("New {provider} session", { provider: label(provider) }), subtitle: here.name, group: t("Commands"), provider, suggested: provider !== "terminal", keywords: "new create 新建", run: () => actions.quickSession(here.id, provider) });
    items.push({ id: "new:options", title: t("New session with options…"), subtitle: here.name, group: t("Commands"), icon: "plus", hint: hint("new-session"), keywords: "new create 新建", run: () => actions.newSession(here.id) });
  }
  items.push(
    { id: "page:sessions", title: t("All sessions"), group: t("Go to"), icon: "clock", hint: hint("sessions"), suggested: true, run: () => actions.openPage("sessions") },
    { id: "page:system", title: t("System"), subtitle: t("CPU, memory, disks and network"), group: t("Go to"), icon: "gauge", hint: hint("system"), suggested: true, keywords: "cpu memory disk network resources 资源 磁盘 内存", run: () => actions.openPage("system") },
    { id: "page:usage", title: t("Usage"), subtitle: t("Tokens, cost and subscription allowance"), group: t("Go to"), icon: "chart", suggested: true, keywords: "token cost usage quota 用量 费用 额度 统计", run: () => actions.openPage("usage") },
    { id: "page:canvas", title: t("Workspace canvas"), group: t("Go to"), icon: "grid", hint: hint("canvas"), run: actions.openCanvas },
  );
  for (const [section, title, icon] of [["preferences", "Preferences", "gauge"], ["agents", "Agent clients", "spark"], ["endpoints", "Endpoint profiles", "settings"], ["accounts", "Official accounts", "account"], ["updates", "Updates", "download"]] as const)
    items.push({ id: `settings:${section}`, title: t(title), subtitle: t("Settings"), group: t("Settings"), icon, hint: section === "preferences" ? hint("settings") : undefined, keywords: "settings 设置", run: () => actions.openSettings(section) });
  for (const workspace of workspaces) items.push({ id: `workspace:${workspace.id}`, title: workspace.name, subtitle: workspace.root_path, group: t("Workspaces"), icon: "folder", run: () => actions.openWorkspace(workspace.id) });
  items.push(
    { id: "add-workspace", title: t("Add workspace"), group: t("Commands"), icon: "plus", run: actions.addWorkspace },
    { id: "toggle-sidebar", title: t("Show or hide the sidebar"), group: t("Commands"), icon: "panelLeft", hint: hint("sidebar"), run: actions.toggleSidebar },
    { id: "toggle-files", title: t("Show or hide the file panel"), group: t("Commands"), icon: "panelRight", hint: hint("files"), run: actions.toggleFiles },
    { id: "theme:light", title: t("Light"), subtitle: t("Appearance"), group: t("Commands"), icon: "gauge", keywords: "theme 主题 外观", run: () => actions.setTheme("light") },
    { id: "theme:dark", title: t("Dark"), subtitle: t("Appearance"), group: t("Commands"), icon: "gauge", keywords: "theme 主题 外观", run: () => actions.setTheme("dark") },
    { id: "theme:system", title: t("Match system"), subtitle: t("Appearance"), group: t("Commands"), icon: "gauge", keywords: "theme 主题 外观", run: () => actions.setTheme("system") },
    { id: "shortcuts", title: t("Keyboard shortcuts"), group: t("Commands"), icon: "info", hint: hint("help"), run: actions.showShortcuts },
  );
  return items;
}
