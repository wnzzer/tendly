/** Local-only subsequence search. No prompts or filenames are sent to a search service. */
export function fuzzyScore(query: string, text: string): number | null {
  const haystack = text.normalize("NFKC").toLocaleLowerCase();
  const tokens = query.normalize("NFKC").toLocaleLowerCase().trim().split(/\s+/).filter(Boolean);
  if (!tokens.length) return 0;
  let score = 0;
  for (const token of tokens) {
    const exact = haystack.indexOf(token);
    if (exact >= 0) {
      score += exact / 100 - token.length * 2;
      continue;
    }
    let from = 0,
      previous = -1;
    for (const character of token) {
      const index = haystack.indexOf(character, from);
      if (index < 0) return null;
      score += previous < 0 ? index / 10 : index - previous - 1;
      previous = index;
      from = index + character.length;
    }
    score += 10;
  }
  return score;
}
export function fuzzyFilter<T>(items: T[], query: string, text: (item: T) => string): T[] {
  return items
    .map((item, index) => ({ item, index, score: fuzzyScore(query, text(item)) }))
    .filter((v): v is { item: T; index: number; score: number } => v.score !== null)
    .sort((a, b) => a.score - b.score || a.index - b.index)
    .map(v => v.item);
}
