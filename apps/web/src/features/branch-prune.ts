/** Deciding when the branch menu quietly refreshes its remote-tracking
 * branches, kept out of the view so the timing can be tested on its own.
 *
 * Opening the menu fires a background `git fetch --prune` so a branch deleted
 * on the remote stops lingering in the list -- Git never prunes those refs on
 * its own, and merely switching branches does not fetch. Fetching touches the
 * network, so it runs at most once in a while per workspace rather than on
 * every open. */

/** No new background fetch for a workspace within this long of the last one. */
export const PRUNE_INTERVAL_MS = 60_000;

/**
 * Whether to fetch for `workspaceId` at `now`, given `last`, when each
 * workspace was last fetched. Returns false and leaves the record untouched
 * when one ran within the interval; otherwise records `now` and returns true.
 *
 * Recording the start -- not the finish -- also keeps two opens in quick
 * succession, or two menus onto the same workspace, from firing at once: only
 * the claim that returns true should run the fetch.
 */
export function claimPrune(last: Map<string, number>, workspaceId: string, now: number): boolean {
  const previous = last.get(workspaceId);
  if (previous !== undefined && now - previous < PRUNE_INTERVAL_MS) return false;
  last.set(workspaceId, now);
  return true;
}
