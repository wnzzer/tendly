import type { TaskState, WorktreeSetup } from "@tendly/protocol";

/**
 * The branch the server names an isolated task when it is given none, for
 * the form to show before it exists: `tendly/` and the title, as far as that
 * spells something in a branch name (`checkouts.rs`, `task_branch`). The
 * server numbers it on when the name is taken, and uses a piece of the
 * session's id when the title has too little to go on.
 */
export function taskBranchPreview(title: string): string {
  let slug = "";
  for (const c of title) {
    if (/^[A-Za-z0-9]$/.test(c)) slug += c.toLowerCase();
    else if (slug && !slug.endsWith("-")) slug += "-";
    if (slug.length >= 40) break;
  }
  slug = slug.replace(/^-+|-+$/g, "");
  return slug.length >= 3 ? `tendly/${slug}` : "tendly/task-…";
}

/** One path per line, as the setup form takes them. */
export function copyLines(text: string): string[] {
  return [...new Set(text.split(/\r?\n/).map(line => line.trim()).filter(Boolean))];
}

/** A setting that prepares nothing is no setting: the server stores none for it. */
export function setupPayload(copy: string, command: string): WorktreeSetup | null {
  const paths = copyLines(copy), run = command.trim();
  return paths.length || run ? { copy: paths, command: run || null } : null;
}

export type SetupSummary = { kind: "none" } | { kind: "some"; copy: string[]; command: string | null };
export function setupSummary(setup: WorktreeSetup | null | undefined): SetupSummary {
  const copy = setup?.copy ?? [], command = setup?.command?.trim() || null;
  return copy.length || command ? { kind: "some", copy, command } : { kind: "none" };
}

/**
 * What finishing a task can do now, from where it stands. A merge needs
 * everything committed and something to merge; with nothing left to merge, the
 * task can simply be put away.
 */
export function taskActions(task: Pick<TaskState, "ahead" | "uncommitted" | "preparing">) {
  const committed = task.uncommitted === 0;
  return {
    merge: !task.preparing && committed && task.ahead > 0,
    finish: !task.preparing && committed && task.ahead === 0,
    discard: !task.preparing,
    push: !task.preparing && task.ahead > 0,
  };
}

/** Added and removed lines over every file that reports them. */
export function taskTotals(task: Pick<TaskState, "files">) {
  let added = 0, removed = 0;
  for (const file of task.files) { added += file.added ?? 0; removed += file.removed ?? 0; }
  return { files: task.files.length, added, removed };
}
