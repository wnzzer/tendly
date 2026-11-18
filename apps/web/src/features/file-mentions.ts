/**
 * Composer suggestions for files in the workspace, typed as `@path`.
 *
 * Unlike a slash command, a mention belongs mid-sentence — "compare @a.ts with
 * @b.ts" — so it is recognised anywhere a word can start, and its term may hold
 * the characters a path holds.
 */
export interface MentionQuery {
  /** The typed path fragment, without the leading `@`. */
  term: string;
  /** Where the `@` sits, so completion replaces from there to the caret. */
  start: number;
  /** The caret, which is where the replaced range ends. */
  end: number;
}

/** Long enough for a nested path, short enough that prose never qualifies. */
const MAX_TERM = 120;

/**
 * A mention is being typed when an `@` opens a word and no whitespace has
 * followed it. `an@example.com` is not one — the `@` there continues a word —
 * and neither is an `@` the caret has already moved past a space from.
 */
export function mentionQuery(text: string, caret: number): MentionQuery | undefined {
  if (caret < 1 || caret > text.length) return undefined;
  const upToCaret = text.slice(0, caret);
  const at = upToCaret.lastIndexOf("@");
  if (at === -1) return undefined;
  // Only an `@` that starts a word: otherwise every email address opens a menu.
  const preceding = at === 0 ? "" : upToCaret[at - 1];
  if (preceding && !/\s/.test(preceding)) return undefined;
  const term = upToCaret.slice(at + 1);
  if (term.length > MAX_TERM || /\s/.test(term)) return undefined;
  return { term, start: at, end: caret };
}

/**
 * Replace the typed fragment with the path and a trailing space.
 *
 * Paths containing spaces are wrapped, so the agent receives one argument
 * rather than a path that ends at the first space.
 */
export function applyMention(text: string, query: MentionQuery, path: string): { text: string; caret: number } {
  const quoted = /\s/.test(path) ? `"${path}"` : path;
  const rest = text.slice(query.end);
  // Completing mid-sentence already has a space after it; adding another would
  // leave a gap that the writer then has to delete.
  const inserted = `@${quoted}${/^\s/.test(rest) ? "" : " "}`;
  return { text: text.slice(0, query.start) + inserted + rest, caret: query.start + inserted.length };
}
