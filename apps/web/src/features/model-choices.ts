import type { NativeModel } from "./chat-model";

/**
 * The models a session can switch to.
 *
 * A client lists the models it knows, named by its own guess (an endpoint ID
 * beginning `claude-fable-5` reads as "Fable 5"), and its official ones —
 * Default, Opus, Sonnet — which behind a gateway either do not exist or only
 * map back to the main model. So once the endpoint has said what it serves,
 * that is the list: the endpoint's models as the endpoint describes them, with
 * the thinking levels the client knows for the same ID, and the
 * model in use if the endpoint does not list it. Without an endpoint catalog,
 * as for an official account, the client's list stands.
 */
export function mergeModels(
  client: readonly NativeModel[],
  endpoint: readonly NativeModel[],
  current?: string,
): NativeModel[] {
  if (!endpoint.length) return [...client];
  // Only the client's thinking levels carry over. Its name and description
  // are its guess at what the ID means ("Opus 5.5 · Best for everyday tasks"),
  // which behind a gateway describes some other model.
  const levels = new Map(client.map(entry => [entry.id, entry.efforts]));
  const withLevels = (entry: NativeModel): NativeModel => {
    const efforts = levels.get(entry.id);
    return efforts ? { ...entry, efforts } : entry;
  };
  const merged = endpoint.map(withLevels);
  if (current && !merged.some(entry => entry.id === current))
    merged.unshift(withLevels({ id: current, name: current }));
  return merged;
}

/**
 * The models a profile offers from its endpoint's list: the ones chosen in
 * it, in that order and named as the endpoint names them, or the whole list
 * when none were chosen. A chosen model the list no longer has stays, by ID.
 */
export function offeredModels<T extends { id: string; name: string }>(
  catalog: readonly T[],
  chosen: readonly string[] | undefined,
): (T | { id: string; name: string })[] {
  if (!chosen?.length) return [...catalog];
  const named = new Map(catalog.map(entry => [entry.id, entry]));
  return chosen.map(id => named.get(id) ?? { id, name: id });
}

/**
 * What a model typed by hand means: an ID as given, or else the ID of the one
 * endpoint model with that name. Sending the name would reach the endpoint as
 * a model it does not have.
 */
export function resolveModel(input: string, endpoint: readonly NativeModel[]): string {
  const typed = input.trim();
  if (!typed || endpoint.some(entry => entry.id === typed)) return typed;
  const named = endpoint.filter(entry => entry.name.toLocaleLowerCase() === typed.toLocaleLowerCase());
  return named.length === 1 ? named[0].id : typed;
}

/** The models whose name or ID contains `query`, all of them for an empty one. */
export function filterModels(models: readonly NativeModel[], query: string): NativeModel[] {
  const term = query.trim().toLocaleLowerCase();
  return term ? models.filter(entry => `${entry.name} ${entry.id}`.toLocaleLowerCase().includes(term)) : [...models];
}

/** Catalog defaults describe new sessions, not the model retained by a resumed CLI. */
export function sessionModel(
  reported: string | undefined,
  ready: boolean,
  resumed: boolean,
  profile: string | undefined,
  catalog: readonly NativeModel[],
): string | undefined {
  if (reported) return reported;
  if (ready || resumed) return undefined;
  return profile ?? catalog.find(entry => entry.isDefault)?.id;
}
