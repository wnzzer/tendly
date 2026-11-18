import type { GitCommit, GitFile } from "@tendly/protocol";

/** Both sides changed it and Git could not combine them: DD, AU, UD, UA, DU, AA or UU. */
export const isConflictedFile = (file: GitFile) =>
  file.index === "U" ||
  file.worktree === "U" ||
  (file.index === "A" && file.worktree === "A") ||
  (file.index === "D" && file.worktree === "D");
export const isStagedFile = (file: GitFile) => file.index !== " " && file.index !== "?" && !isConflictedFile(file);
export const isChangedFile = (file: GitFile) =>
  (file.worktree !== " " || file.index === "?") && !isConflictedFile(file);
export const pathsForGitFiles = (files: GitFile[]) => [
  ...new Set(files.flatMap(file => (file.original_path ? [file.path, file.original_path] : [file.path]))),
];
/** `hunk` numbers the hunk a header row opens, counting from 0. */
export interface DiffRow {
  text: string;
  before: string | number;
  after: string | number;
  kind: "hunk" | "meta" | "added" | "removed" | "context";
  hunk?: number;
}

/** Line numbers are calculated from real unified diff hunk headers. */
export function parseUnifiedDiff(diff: string): DiffRow[] {
  let before = 0,
    after = 0,
    inHunk = false,
    hunks = 0,
    combined = false;
  return diff.split("\n").map(text => {
    // A conflicted file diffs against both sides at once (`diff --cc`): two
    // marker columns, and only the result's line numbers.
    if (text.startsWith("diff --cc")) {
      combined = true;
      inHunk = false;
      return { text, before: "", after: "", kind: "meta" };
    }
    if (combined) {
      const header = text.match(/^@@@ .* \+(\d+)(?:,\d+)? @@@/);
      if (header) {
        after = Number(header[1]);
        inHunk = true;
        return { text, before: "", after: "", kind: "hunk" };
      }
      if (!inHunk) return { text, before: "", after: "", kind: "meta" };
      const marks = text.slice(0, 2);
      if (marks.includes("-")) return { text, before: "", after: "", kind: "removed" };
      if (marks.includes("+")) return { text, before: "", after: after++, kind: "added" };
      return { text, before: "", after: text ? after++ : "", kind: text ? "context" : "meta" };
    }
    const match = text.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
    if (match) {
      before = Number(match[1]);
      after = Number(match[2]);
      inHunk = true;
      return { text, before: "", after: "", kind: "hunk", hunk: hunks++ };
    }
    if (!inHunk || text.startsWith("diff --git")) {
      if (text.startsWith("diff --git")) inHunk = false;
      return { text, before: "", after: "", kind: "meta" };
    }
    if (text.startsWith("+")) return { text, before: "", after: after++, kind: "added" };
    if (text.startsWith("-")) return { text, before: before++, after: "", kind: "removed" };
    if (text.startsWith(" ")) return { text, before: before++, after: after++, kind: "context" };
    return { text, before: "", after: "", kind: "meta" };
  });
}

/**
 * The patch for one hunk of a diff: its file's header and that hunk alone,
 * which `git apply` takes to stage, unstage or discard just that part.
 * Hunks are counted as `parseUnifiedDiff` counts them.
 */
export function hunkPatch(diff: string, index: number): string | undefined {
  const lines = diff.split("\n");
  if (lines[lines.length - 1] === "") lines.pop();
  let header: string[] = [],
    inHeader = false,
    hunk = -1,
    taken: string[] | undefined;
  for (const line of lines) {
    if (line.startsWith("diff --git")) {
      if (taken) break;
      header = [line];
      inHeader = true;
      continue;
    }
    if (line.startsWith("@@")) {
      if (taken) break;
      inHeader = false;
      hunk++;
      if (hunk === index) taken = [...header, line];
      continue;
    }
    if (inHeader) header.push(line);
    else if (taken) taken.push(line);
  }
  return taken && taken.length > 1 ? taken.join("\n") + "\n" : undefined;
}

export type RefKind = "head" | "branch" | "remote" | "tag";
export interface CommitRef {
  kind: RefKind;
  name: string;
}
/** Git's decorations, `HEAD -> main, origin/main, tag: v1`, as badges. */
export function commitRefs(refs: string[], remotes: string[] = []): CommitRef[] {
  return refs.flatMap<CommitRef>(raw => {
    if (raw.startsWith("tag: ")) return [{ kind: "tag", name: raw.slice(5) }];
    if (raw.startsWith("HEAD -> ")) return [{ kind: "head", name: raw.slice(8) }];
    if (raw === "HEAD") return [{ kind: "head", name: "HEAD" }];
    if (raw.endsWith("/HEAD") || raw === "refs/stash") return [];
    const remote = remotes.length
      ? remotes.some(name => raw.startsWith(name + "/"))
      : raw.includes("/") && raw.startsWith("origin/");
    return [{ kind: remote ? "remote" : "branch", name: raw }];
  });
}

/**
 * Lanes for drawing history as a graph, one row per commit, in the order
 * Git listed them (children before parents). `lanes` and `next` are the
 * columns occupied entering and leaving the row; `into` are the columns that
 * end at this commit, `out` the columns its parents continue in.
 */
export interface GraphRow {
  column: number;
  lanes: number[];
  next: number[];
  into: number[];
  out: number[];
  width: number;
}
export function commitGraph(commits: Pick<GitCommit, "hash" | "parents">[]): GraphRow[] {
  const lanes: (string | null)[] = [];
  const free = () => {
    const index = lanes.indexOf(null);
    return index < 0 ? lanes.length : index;
  };
  const occupied = () => lanes.flatMap((hash, index) => (hash ? [index] : []));
  return commits.map(commit => {
    const entering = occupied();
    let column = lanes.indexOf(commit.hash);
    // A branch tip nothing above leads to starts a lane of its own.
    const tip = column < 0;
    if (tip) {
      column = free();
      lanes[column] = commit.hash;
    }
    const into = lanes.flatMap((hash, index) => (hash === commit.hash && !(tip && index === column) ? [index] : []));
    for (const index of into) lanes[index] = null;
    const out: number[] = [];
    commit.parents.forEach((parent, order) => {
      const existing = lanes.indexOf(parent);
      if (existing >= 0) {
        out.push(existing);
        return;
      }
      const index = order === 0 ? column : free();
      lanes[index] = parent;
      out.push(index);
    });
    while (lanes.length && lanes[lanes.length - 1] === null) lanes.pop();
    const next = occupied();
    return {
      column,
      lanes: entering,
      next,
      into,
      out,
      width: Math.max(column + 1, lanes.length, ...entering.map(i => i + 1)),
    };
  });
}

/** Seconds since the epoch, as "5 min ago" or a date. */
export function relativeTime(
  seconds: number,
  t: (text: string, values?: Record<string, string | number>) => string,
  now = Date.now(),
): string {
  const minutes = Math.max(0, Math.round((now / 1000 - seconds) / 60));
  if (minutes < 1) return t("just now");
  if (minutes < 60) return t("{n} min ago", { n: minutes });
  const hours = Math.round(minutes / 60);
  if (hours < 24) return t("{n} h ago", { n: hours });
  const days = Math.round(hours / 24);
  if (days < 30) return t("{n} d ago", { n: days });
  return new Date(seconds * 1000).toLocaleDateString();
}
