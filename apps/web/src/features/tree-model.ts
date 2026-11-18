import type { FileEntry } from "@tendly/protocol";

export interface TreeDirectory {
  entries: FileEntry[];
  loaded: boolean;
  loading: boolean;
  error: string;
}

export interface TreeRow {
  entry: FileEntry;
  parent: string;
  depth: number;
  position: number;
  siblings: number;
  expanded: boolean;
}

/** The API speaks workspace-relative POSIX paths, even when the UI runs elsewhere. */
export function treeAncestors(path: string): string[] {
  const parts = path.split("/");
  if (!path || path.includes("\0") || parts.some(part => !part || part === "." || part === "..")) {
    throw new Error("Invalid workspace-relative file path.");
  }
  return parts.map((_, index) => parts.slice(0, index).join("/"));
}

export function treeParent(path: string): string {
  return path.slice(0, Math.max(0, path.lastIndexOf("/")));
}

export function isTreeChild(parent: string, path: string): boolean {
  const prefix = parent ? `${parent}/` : "";
  const name = path.slice(prefix.length);
  return (
    path.startsWith(prefix) && !!name && !name.includes("/") && name !== "." && name !== ".." && !name.includes("\0")
  );
}

const collator = new Intl.Collator("en", { numeric: true, sensitivity: "base" });
export function sortTreeEntries(entries: readonly FileEntry[]): FileEntry[] {
  return [...entries].sort(
    (a, b) =>
      Number(b.kind === "directory") - Number(a.kind === "directory") ||
      collator.compare(a.name, b.name) ||
      a.path.localeCompare(b.path),
  );
}

/** Project the cached hierarchy only: filtering never crawls the filesystem. */
export function flattenTree(
  directories: ReadonlyMap<string, TreeDirectory>,
  expanded: ReadonlySet<string>,
  query = "",
): TreeRow[] {
  const term = query.trim().toLocaleLowerCase();
  function project(parent: string, depth: number): TreeRow[] {
    const branches: TreeRow[][] = [];
    for (const entry of directories.get(parent)?.entries ?? []) {
      // Never recurse into symlinks or malformed/self-referential API entries.
      if (!isTreeChild(parent, entry.path)) continue;
      const isDirectory = entry.kind === "directory";
      const children = isDirectory && (term || expanded.has(entry.path)) ? project(entry.path, depth + 1) : [];
      if (term && !entry.path.toLocaleLowerCase().includes(term) && !children.length) continue;
      branches.push([
        {
          entry,
          parent,
          depth,
          position: 0,
          siblings: 0,
          expanded: isDirectory && (expanded.has(entry.path) || (!!term && !!children.length)),
        },
        ...children,
      ]);
    }
    return branches.flatMap((branch, index) => {
      branch[0].position = index + 1;
      branch[0].siblings = branches.length;
      return branch;
    });
  }
  return project("", 0);
}

/** What a key does on a row of the tree; the explorer carries it out. */
export type TreeKeyAction =
  { kind: "open" } | { kind: "focus"; path: string } | { kind: "expand" } | { kind: "collapse"; path: string };

/**
 * The file tree's keys, as a tree view's: up and down move by row, Home and
 * End to the ends, right opens a folder and then steps into it, left closes
 * an open folder and otherwise goes to the parent, Enter and Space open.
 * Keys with a modifier, and any other key, are not the tree's.
 */
export function treeKeyAction(
  key: string,
  modifier: boolean,
  rows: readonly TreeRow[],
  row: TreeRow,
): TreeKeyAction | "ignore" | "none" {
  if (modifier || !["ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight", "Home", "End", "Enter", " "].includes(key))
    return "ignore";
  const index = rows.findIndex(item => item.entry.path === row.entry.path);
  const focus = (target: TreeRow | undefined): TreeKeyAction | "none" =>
    target ? { kind: "focus", path: target.entry.path } : "none";
  if (key === "Enter" || key === " ") return { kind: "open" };
  if (key === "ArrowDown") return focus(rows[index + 1]);
  if (key === "ArrowUp") return focus(rows[index - 1]);
  if (key === "Home") return focus(rows[0]);
  if (key === "End") return focus(rows.at(-1));
  if (key === "ArrowRight") {
    if (row.entry.kind !== "directory") return "none";
    if (!row.expanded) return { kind: "expand" };
    const child = rows[index + 1];
    return child?.parent === row.entry.path ? focus(child) : "none";
  }
  // ArrowLeft
  if (row.entry.kind === "directory" && row.expanded) return { kind: "collapse", path: row.entry.path };
  return row.parent ? { kind: "focus", path: row.parent } : "none";
}
