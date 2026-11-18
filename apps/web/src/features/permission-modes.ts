/**
 * How tools get approved, for the running session.
 *
 * The list comes from the client — Codex has no plan or accept-edits — and the
 * current value is the one the client reports rather than the one last asked
 * for, so this never claims a mode the client is not in.
 */
type Translate = (text: string, values?: Record<string, string | number>) => string;

export const PERMISSION_LABELS: Record<string, string> = {
  ask: "Ask every time",
  plan: "Plan first",
  accept_edits: "Accept edits",
  danger: "Never ask",
};

/**
 * What each mode actually does, next to its name.
 *
 * The names are short enough to be guessed at wrongly -- "accept edits" does
 * not say that everything else still asks -- and this is the one place where
 * there is room to say it. The unattended mode says what it costs rather than
 * what it saves, because that is the part worth reading twice.
 */
export const PERMISSION_NOTES: Record<string, string> = {
  ask: "Asks before it runs anything.",
  plan: "Works out an approach first and waits for you to accept it.",
  accept_edits: "File edits go through; everything else still asks.",
  danger: "Runs every tool without asking. Anyone who can reach this workspace can too.",
};

/** The chip's name for the mode in use: a mode this does not know is named as the client named it. */
export function permissionName(mode: string, t: Translate): string {
  return mode ? t(PERMISSION_LABELS[mode] ?? mode) : t("Approval");
}
