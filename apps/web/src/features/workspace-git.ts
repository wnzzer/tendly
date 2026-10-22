import { computed, ref, type Ref } from "vue";
import type { GitStatus, Session } from "@tendly/protocol";
import { ApiConnectionError, ApiError, request } from "./api";
import { placeKey, placeOf, placeUrl } from "./checkout-place";

/**
 * The Git status of every workspace on screen, as the shell keeps it for the
 * status bar, the drawer and the tab strip, and the counters that tell file
 * trees and Git views to reload. Each is kept per place (`placeKey`): a
 * workspace directory, or a worktree a session works in.
 */

/** A counter record with one workspace's count moved on, as a new object so whatever reads it sees the change. */
export function bumped(counter: Readonly<Record<string, number>>, id: string): Record<string, number> { return { ...counter, [id]: (counter[id] ?? 0) + 1 }; }

/** Each workspace directory's current branch, for those on one; a worktree's is its session's own. */
export function branchesByWorkspace(statuses: Readonly<Record<string, GitStatus>>): Record<string, string> {
  return Object.fromEntries(Object.entries(statuses).flatMap(([id, status]) => status.branch && !placeOf(id).checkout ? [[id, status.branch]] : []));
}

/** A failed status read worth reporting: the host is unreachable or the sign-in lapsed. Anything else just means no Git here. */
export function gitFailureMatters(cause: unknown): boolean { return cause instanceof ApiConnectionError || (cause instanceof ApiError && cause.status === 401); }

export interface WorkspaceGitOptions {
  /** The place the status bar and the drawer show. */
  contextPlace: Readonly<Ref<string | undefined>>;
  activeSessions: Readonly<Ref<Session[]>>;
  /** Signed out: nothing is asked of the server. */
  paused: () => boolean;
  report: (cause: unknown) => void;
}

export function useWorkspaceGit({ contextPlace, activeSessions, paused, report }: WorkspaceGitOptions) {
  const gitStatuses = ref<Record<string, GitStatus>>({}), gitAvailable = ref<Record<string, boolean>>({});
  /** Bumped to reload a workspace's file tree, and its Git views. */
  const fileRefresh = ref<Record<string, number>>({}), gitRefresh = ref<Record<string, number>>({});
  const currentGit = computed<GitStatus>(() => gitStatuses.value[contextPlace.value ?? ""] ?? { branch: null, files: [] });
  const currentGitAvailable = computed(() => !!gitAvailable.value[contextPlace.value ?? ""]);
  /** Whether the place's status has been read at all: until then it is neither a branch nor "unavailable". */
  const currentGitKnown = computed(() => (contextPlace.value ?? "") in gitAvailable.value);
  const workspaceBranches = computed(() => branchesByWorkspace(gitStatuses.value));
  const gitLoading = new Set<string>();
  function tick(counter: typeof fileRefresh, id: string) { counter.value = bumped(counter.value, id); }
  function gitChanged(id: string, status: GitStatus) { gitStatuses.value = { ...gitStatuses.value, [id]: status }; gitAvailable.value = { ...gitAvailable.value, [id]: true }; }
  async function refreshGit(id: string) {
    if (!id || paused() || gitLoading.has(id)) return;
    gitLoading.add(id);
    try {
      const result = await request<GitStatus>(placeUrl(id, "/git/status"));
      const changed = JSON.stringify(gitStatuses.value[id]) !== JSON.stringify(result);
      gitChanged(id, result);
      // An agent at work may change a diff without changing the status line, so its views reload anyway.
      if (changed || activeSessions.value.some(session => placeKey(session.workspace_id, session.checkout_path) === id)) tick(gitRefresh, id);
    } catch (cause) { gitAvailable.value = { ...gitAvailable.value, [id]: false }; if (gitFailureMatters(cause)) report(cause); }
    finally { gitLoading.delete(id); }
  }
  /** Files were written in a workspace: its tree, its Git views and its status all catch up. */
  function filesSaved(id: string) { tick(fileRefresh, id); tick(gitRefresh, id); void refreshGit(id); }

  return { gitStatuses, gitAvailable, fileRefresh, gitRefresh, currentGit, currentGitAvailable, currentGitKnown, workspaceBranches, tick, gitChanged, refreshGit, filesSaved };
}
