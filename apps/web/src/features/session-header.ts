import { reactive } from "vue";

/**
 * What a phone's top bar shows for the session in a pane.
 *
 * A phone composer has room for the controls a message needs and no more, so
 * the branch a session works on is named in the top bar instead, as it is in
 * other phone chat apps. The pane owns the branch and its picker; the shell
 * owns the bar. The pane registers here, and the bar reads what the focused
 * pane registered.
 */
export interface HeaderBranch {
  label: string;
  /** The session works in its own worktree rather than the workspace directory. */
  worktree: boolean;
  disabled: boolean;
  toggle: () => void;
}

export const headerBranches = reactive(new Map<string, HeaderBranch>());
