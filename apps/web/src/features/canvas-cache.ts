import type { LayoutDocument, Workspace } from "@tendly/protocol";
import { validateLayout } from "../layout/layout-engine";
type CanvasStorage = Pick<Storage, "getItem" | "setItem">;
export interface CachedCanvas {
  layout: LayoutDocument;
  pending: boolean;
  updated_at: string;
}

/** serde_json can reorder object keys; array order and all layout values still matter. */
export function sameCanvasLayout(first: LayoutDocument, second: LayoutDocument): boolean {
  const canonical = (value: unknown): unknown =>
    Array.isArray(value)
      ? value.map(canonical)
      : value && typeof value === "object"
        ? Object.fromEntries(
            Object.entries(value)
              .sort(([a], [b]) => a.localeCompare(b))
              .map(([key, entry]) => [key, canonical(entry)]),
          )
        : value;
  return JSON.stringify(canonical(first)) === JSON.stringify(canonical(second));
}

/** The oldest registration is stable when new workspaces are added. Different
 * deployments on the same origin have different workspace UUIDs. */
export function canvasStorageKey(origin: string, workspaces: Workspace[]): string | undefined {
  const anchor = [...workspaces].sort(
    (a, b) => a.created_at.localeCompare(b.created_at) || a.id.localeCompare(b.id),
  )[0];
  return anchor ? `tendly:shared-canvas:${encodeURIComponent(origin)}:${anchor.id}` : undefined;
}
export function readCanvasCache(storage: CanvasStorage | undefined, key: string | undefined): CachedCanvas | undefined {
  if (!storage || !key) return undefined;
  try {
    const entry = JSON.parse(storage.getItem(key) ?? "null");
    if (!entry || !validateLayout(entry.layout) || typeof entry.pending !== "boolean") return undefined;
    return entry;
  } catch {
    return undefined;
  }
}
export function writeCanvasCache(
  storage: CanvasStorage | undefined,
  key: string | undefined,
  layout: LayoutDocument,
  pending: boolean,
): boolean {
  if (!storage || !key || !validateLayout(layout)) return false;
  try {
    storage.setItem(key, JSON.stringify({ layout, pending, updated_at: new Date().toISOString() }));
    return true;
  } catch {
    return false;
  }
}
export function browserStorage(): CanvasStorage | undefined {
  try {
    return typeof window !== "undefined" ? window.localStorage : undefined;
  } catch {
    return undefined;
  }
}
