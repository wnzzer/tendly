import type { TreeDirectory } from "./tree-model";

/**
 * The paths behind adding and renaming a file.
 *
 * Typing a name is not the same as naming a path: people type spaces they did
 * not mean, a leading slash out of habit from the shell, and a folder they want
 * created along the way. These turn what was typed into the one path the host
 * is asked for, so the request means what the person meant.
 */

/** Where a new file goes: under the folder in hand, at the path typed. */
export function newFilePath(base: string, typed: string): string {
  const name = typed
    .trim()
    .replace(/^\/+/, "")
    .replace(/\/+$/, "")
    .replace(/\/{2,}/g, "/");
  if (!name) return "";
  const parent = base.replace(/\/+$/, "");
  return parent ? `${parent}/${name}` : name;
}

/**
 * Where a rename goes: the same folder, a new last name.
 *
 * A path typed into a rename is still a rename of the last segment, because the
 * row being renamed is the one in that folder. Moving elsewhere is a different
 * act and is not what the tree offered.
 */
export function renamedPath(path: string, typed: string): string {
  const name = typed.trim().replace(/^\/+|\/+$/g, "");
  if (!name || name.includes("/")) return "";
  const cut = path.lastIndexOf("/");
  return cut === -1 ? name : `${path.slice(0, cut)}/${name}`;
}

/**
 * Whether this folder already holds this name, as far as the tree knows.
 *
 * A folder it has not loaded knows nothing, and says so by answering false: the
 * host is what actually refuses a taken name, and this only spares the person a
 * round trip to be told something the tree could already see.
 */
export function nameTaken(directories: Map<string, TreeDirectory>, path: string): boolean {
  const cut = path.lastIndexOf("/");
  const state = directories.get(cut === -1 ? "" : path.slice(0, cut));
  return !!state?.loaded && state.entries.some(entry => entry.path === path);
}
