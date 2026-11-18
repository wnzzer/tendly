import { reactive } from "vue";

export type GitView = "changes" | "history" | "stashes";
/** `historyPath` narrows history to one file; `historyAll` shows every branch. */
export interface GitViewState {
  message: string;
  amend?: boolean;
  selected?: { path: string; staged: boolean };
  view?: GitView;
  historyAll?: boolean;
  historyPath?: string;
  commit?: string;
  stash?: string;
}
const views = reactive(new Map<string, GitViewState>());
/** UI drafts must survive the layout projecting a split into tabs. */
export function gitViewState(workspaceId: string): GitViewState {
  if (!views.has(workspaceId)) views.set(workspaceId, { message: "" });
  return views.get(workspaceId)!;
}
export function hasGitDrafts() {
  return [...views.values()].some(view => view.message.trim().length > 0);
}
