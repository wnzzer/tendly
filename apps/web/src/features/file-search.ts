import { request } from "./api";
import { placeUrl } from "./checkout-place";

export interface SearchHit { path: string; name: string; kind: "file" | "directory"; score: number; indices: number[] }
export interface SearchResults { files: SearchHit[]; truncated: boolean; available: boolean; timed_out?: boolean }

/** Long enough that a typed word is one query, short enough to feel immediate. */
export const SEARCH_DEBOUNCE_MS = 220;

/**
 * Workspace-wide search, one request at a time.
 *
 * Each query supersedes the one before it: the index has no cancellation, so an
 * in-flight request is abandoned rather than awaited, and a slow answer for an
 * earlier query must never overwrite the results for what is typed now.
 */
export function createFileSearch(run: (workspace: string, query: string, signal: AbortSignal) => Promise<SearchResults> = requestSearch) {
  let controller: AbortController | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let generation = 0;

  function cancel() {
    if (timer !== undefined) { clearTimeout(timer); timer = undefined; }
    controller?.abort();
    controller = undefined;
    generation += 1; // Invalidate any reply still on its way.
  }

  function search(workspace: string, query: string, apply: (results: SearchResults | undefined) => void) {
    cancel();
    if (!query.trim()) { apply(undefined); return; }
    const mine = generation;
    timer = setTimeout(() => {
      timer = undefined;
      controller = new AbortController();
      run(workspace, query, controller.signal)
        .then(results => { if (mine === generation) apply(results); })
        // A superseded or failed query leaves the previous view alone rather
        // than flashing an empty list the user would read as "no matches".
        .catch(() => { if (mine === generation) apply(undefined); });
    }, SEARCH_DEBOUNCE_MS);
  }

  return { search, cancel };
}

async function requestSearch(workspace: string, query: string, signal: AbortSignal): Promise<SearchResults> {
  const params = new URLSearchParams({ q: query, limit: "20" });
  return await request<SearchResults>(placeUrl(workspace, `/files/search?${params}`), { signal });
}

/**
 * Split a name around the positions the index matched, so highlighting shows
 * the same characters that produced the ranking instead of a second guess.
 * `indices` are offsets into the full path; only those inside the name segment
 * are usable, and out-of-range values are ignored rather than trusted.
 */
export function highlight(path: string, name: string, indices: readonly number[]) {
  const offset = path.length - name.length;
  const marked = new Set(indices.filter(index => index >= offset && index < path.length).map(index => index - offset));
  const parts: { text: string; match: boolean }[] = [];
  for (const [index, character] of [...name].entries()) {
    const match = marked.has(index);
    const last = parts[parts.length - 1];
    if (last && last.match === match) last.text += character;
    else parts.push({ text: character, match });
  }
  return parts;
}
