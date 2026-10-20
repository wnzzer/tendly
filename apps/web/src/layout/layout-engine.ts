import type { CollapsedLayout, LayoutDocument, LayoutNode, LeafPaneKind, PaneNode, SplitDirection, SplitNode, StackNode } from "@tendly/protocol/layout";

export type { CollapsedLayout, LayoutDocument, LayoutNode, LeafPaneKind, PaneKind, PaneNode, SplitDirection, SplitNode, StackNode } from "@tendly/protocol/layout";

/** Layout owns placement, never the lifetime of a session, editor, or process. */
export const DEFAULT_LAYOUT_VERSION = 1;
export const SNAP_RATIOS = [0.25, 1 / 3, 0.5, 2 / 3, 0.75] as const;
export const SPLIT_GAP = 6;
export const MIN_PANE_WIDTH = 280;
export const MIN_PANE_HEIGHT = 180;
export type DockPosition = "center" | "left" | "right" | "top" | "bottom";
export type LayoutPreset = "1:1" | "2x2" | "1:2:1";
const LEAF_KINDS: readonly LeafPaneKind[] = ["agent_chat", "editor", "terminal", "git_diff", "file_preview"];

export function createPane(id: string, kind: LeafPaneKind, title?: string, metadata?: Record<string, unknown>): PaneNode {
  return { type: "pane", id, kind, ...(title ? { title } : {}), ...(metadata ? { metadata: { ...metadata } } : {}) };
}

export function createDefaultLayoutDocument(): LayoutDocument {
  return { version: DEFAULT_LAYOUT_VERSION, root: {
    type: "split", id: "root", direction: "horizontal", ratio: 0.58,
    first: createPane("agent-chat", "agent_chat", "Agent session"),
    second: createPane("changes", "git_diff", "Changes"),
  } };
}

/** Free resize while dragging; snapping is a separate release-time action. */
export function normalizeRatio(value: number): number {
  return Number.isFinite(value) ? Math.min(0.95, Math.max(0.05, value)) : 0.5;
}
export function snapRatio(value: number, threshold = 0.025): number {
  const clamped = normalizeRatio(value);
  const nearest = SNAP_RATIOS.reduce<number>((best, candidate) => Math.abs(candidate - clamped) < Math.abs(best - clamped) ? candidate : best, SNAP_RATIOS[0]);
  return Math.abs(nearest - clamped) <= threshold ? nearest : clamped;
}
function isDocument(value: unknown): value is LayoutDocument {
  return !!value && typeof value === "object" && "root" in value && "version" in value;
}
export function cloneNode(node: LayoutNode): LayoutNode {
  if (node.type === "split") return { ...node, first: cloneNode(node.first), second: cloneNode(node.second) };
  if (node.type === "stack") return { ...node, panes: node.panes.map((pane) => cloneNode(pane) as PaneNode) };
  return { ...node, ...(node.metadata ? { metadata: { ...node.metadata } } : {}) };
}

export type NodeMatcher = string | ((node: LayoutNode) => boolean);
export function findNodes(root: LayoutNode, matcher?: NodeMatcher): LayoutNode[] {
  const result: LayoutNode[] = [];
  const visit = (node: LayoutNode) => {
    if (matcher === undefined || (typeof matcher === "string" ? node.id === matcher : matcher(node))) result.push(node);
    if (node.type === "split") { visit(node.first); visit(node.second); }
    else if (node.type === "stack") node.panes.forEach(visit);
  };
  visit(root);
  return result;
}
export function findNode(root: LayoutNode, id: string): LayoutNode | undefined { return findNodes(root, id)[0]; }
export function flattenPanes(root: LayoutNode): PaneNode[] { return findNodes(root, (node) => node.type === "pane") as PaneNode[]; }
function newId(base: string, occupied: Set<string>): string {
  let id = base;
  let suffix = 1;
  while (occupied.has(id)) id = `${base}-${++suffix}`;
  occupied.add(id);
  return id;
}
function idsOf(root: LayoutNode): Set<string> { return new Set(findNodes(root).map((node) => node.id)); }
function emptyStack(id: string): StackNode { return { type: "stack", kind: "stack", id, panes: [] }; }

/** Operations on a tab target its containing stack, not a fake nested leaf. */
export function containerId(root: LayoutNode, paneOrNodeId: string): string {
  const stack = findNodes(root, (node) => node.type === "stack" && node.panes.some((pane) => pane.id === paneOrNodeId))[0];
  return stack?.id ?? paneOrNodeId;
}
function replaceNode(root: LayoutNode, id: string, transform: (node: LayoutNode) => LayoutNode): LayoutNode {
  if (root.id === id) return transform(root);
  if (root.type !== "split") return root;
  const first = replaceNode(root.first, id, transform);
  const second = replaceNode(root.second, id, transform);
  return first === root.first && second === root.second ? root : { ...root, first, second };
}
export function activatePane(root: LayoutNode, id: string): LayoutNode {
  if (root.type === "stack") return root.activePaneId !== id && root.panes.some((pane) => pane.id === id) ? { ...root, activePaneId: id } : root;
  if (root.type !== "split") return root;
  const first = activatePane(root.first, id);
  const second = activatePane(root.second, id);
  return first === root.first && second === root.second ? root : { ...root, first, second };
}

/** Split ancestors remember their last selected descendant even while a larger
 * ancestor is collapsed. This keeps tab choice stable across breakpoints. */
export function selectionGroups(root: LayoutNode, paneId: string): string[] {
  const result: string[] = [];
  const visit = (node: LayoutNode): boolean => {
    if (node.type === "pane") return node.id === paneId;
    if (node.type === "stack") return node.panes.some((pane) => pane.id === paneId);
    const first = visit(node.first);
    const second = visit(node.second);
    if (first || second) result.push(node.id);
    return first || second;
  };
  visit(root);
  return result;
}
export function updatePane(root: LayoutNode, pane: PaneNode): LayoutNode {
  if (root.type === "pane") return root.id === pane.id ? { ...root, ...pane } : root;
  if (root.type === "stack") return root.panes.some((item) => item.id === pane.id) ? { ...root, panes: root.panes.map((item) => item.id === pane.id ? { ...item, ...pane } : item) } : root;
  const first = updatePane(root.first, pane);
  const second = updatePane(root.second, pane);
  return first === root.first && second === root.second ? root : { ...root, first, second };
}

/** Bind an unbound pane in place without introducing an extra placeholder tab.
 * Unlike updatePane, this deliberately changes identity and fixes tab selection. */
export function replacePane(root: LayoutNode, id: string, replacement: PaneNode): LayoutNode {
  if (!isPaneNode(replacement) || (replacement.id !== id && findNode(root, replacement.id))) return root;
  const replace = (node: LayoutNode): LayoutNode => {
    if (node.type === "pane") return node.id === id ? replacement : node;
    if (node.type === "stack") {
      if (!node.panes.some((pane) => pane.id === id)) return node;
      return { ...node, panes: node.panes.map((pane) => pane.id === id ? replacement : pane), activePaneId: node.activePaneId === id ? replacement.id : node.activePaneId };
    }
    const first = replace(node.first);
    const second = replace(node.second);
    return first === node.first && second === node.second ? node : { ...node, first, second };
  };
  return replace(root);
}
export function splitPane(root: LayoutNode, id: string, direction: SplitDirection, ratio = 0.5): LayoutNode {
  const occupied = idsOf(root);
  const targetId = containerId(root, id);
  return replaceNode(root, targetId, (node) => ({
    type: "split", id: newId(`${targetId}:split`, occupied), direction, ratio: normalizeRatio(ratio),
    first: node, second: emptyStack(newId(`${targetId}:empty`, occupied)),
  }));
}
export function resizeSplit(root: LayoutNode, id: string, ratio: number): LayoutNode {
  return replaceNode(root, id, (node) => node.type === "split" ? { ...node, ratio: normalizeRatio(ratio) } : node);
}

/** The panes and tab groups the splits hold, left to right and top to bottom. */
export function paneGroups(root: LayoutNode): LayoutNode[] {
  return root.type === "split" ? [...paneGroups(root.first), ...paneGroups(root.second)] : [root];
}
/** Two pane groups trade places; their sizes stay with the places. */
export function swapGroups(root: LayoutNode, firstId: string, secondId: string): LayoutNode {
  const first = findNode(root, firstId), second = findNode(root, secondId);
  if (!first || !second || first === second || first.type === "split" || second.type === "split") return root;
  const swap = (node: LayoutNode): LayoutNode => {
    if (node.id === firstId) return second;
    if (node.id === secondId) return first;
    if (node.type !== "split") return node;
    const a = swap(node.first), b = swap(node.second);
    return a === node.first && b === node.second ? node : { ...node, first: a, second: b };
  };
  return swap(root);
}
/**
 * Moves the divider nearest to `id` that runs across `direction`, as tmux's
 * resize-pane does: left and up move it towards the start, right and down
 * towards the end, whichever side of it the pane is on.
 */
export function nudgeDivider(root: LayoutNode, id: string, direction: "left" | "right" | "up" | "down", step = 0.05): LayoutNode {
  const axis: SplitDirection = direction === "left" || direction === "right" ? "horizontal" : "vertical";
  const path: SplitNode[] = [];
  const visit = (node: LayoutNode): boolean => {
    if (node.id === id) return true;
    if (node.type !== "split") return false;
    if (visit(node.first) || visit(node.second)) { path.push(node); return true; }
    return false;
  };
  if (!visit(root)) return root;
  const split = path.find(node => node.direction === axis);
  if (!split) return root;
  const delta = direction === "left" || direction === "up" ? -step : step;
  return resizeSplit(root, split.id, Math.min(0.9, Math.max(0.1, split.ratio + delta)));
}

/** Removing a view must never stop or delete the underlying session.
 * `recent` lists pane ids oldest-first; when given, a stack whose active tab
 * closes returns to the tab used before it rather than its neighbour. */
export function closePane(root: LayoutNode, paneId: string, recent: readonly string[] = []): LayoutNode {
  const remove = (node: LayoutNode): LayoutNode | null => {
    if (node.type === "pane") return node.id === paneId ? null : node;
    if (node.type === "stack") {
      if (node.id === paneId && !node.panes.length) return null;
      const index = node.panes.findIndex((pane) => pane.id === paneId);
      if (index < 0) return node;
      const panes = node.panes.filter((pane) => pane.id !== paneId);
      if (!panes.length) return null;
      if (panes.some((pane) => pane.id === node.activePaneId)) return { ...node, panes };
      const previous = [...recent].reverse().find((id) => id !== paneId && panes.some((pane) => pane.id === id));
      return { ...node, panes, activePaneId: previous ?? panes[Math.min(index, panes.length - 1)].id };
    }
    const first = remove(node.first);
    const second = remove(node.second);
    if (!first) return second;
    if (!second) return first;
    return first === node.first && second === node.second ? node : { ...node, first, second };
  };
  return remove(root) ?? emptyStack(newId("empty-workspace", idsOf(root)));
}

/** Move an existing pane, or insert a new session/file. Center = tab; edge = split. */
export function dockPane(root: LayoutNode, incoming: PaneNode, targetId: string, position: DockPosition): LayoutNode {
  if (!isPaneNode(incoming)) return root;
  const target = findNode(root, containerId(root, targetId));
  if (!target) return root;
  const existing = findNode(root, incoming.id);
  const pane = existing?.type === "pane" ? existing : incoming;
  if (target.type === "pane" && target.id === pane.id) return root;
  if (position === "center" && target.type === "stack" && target.panes.some((item) => item.id === pane.id)) return activatePane(root, pane.id);
  const fallbackIds = flattenPanes(target).filter((item) => item.id !== pane.id).map((item) => item.id);
  let next = existing ? closePane(root, pane.id) : root;
  let destination = findNode(next, target.id);
  if (!destination) {
    const fallback = fallbackIds.find((id) => findNode(next, id));
    destination = fallback ? findNode(next, containerId(next, fallback)) : next;
  }
  // A projected collapsed split is not a canonical tab group. Insert into a
  // leaf, leaving the original split/ratios intact for a wider viewport.
  if (position === "center" && destination?.type === "split") {
    let leaf: LayoutNode = destination;
    while (leaf.type === "split") leaf = leaf.first;
    destination = leaf;
  }
  const occupied = idsOf(next);
  occupied.add(pane.id);
  next = replaceNode(next, destination?.id ?? next.id, (node) => {
    if (position === "center") {
      if (node.type === "stack") return { ...node, panes: [...node.panes, pane], activePaneId: pane.id };
      if (node.type === "pane") return { type: "stack", kind: "stack", id: newId(`${node.id}:tabs`, occupied), panes: [node, pane], activePaneId: pane.id };
      return node;
    }
    const leading = position === "left" || position === "top";
    return {
      type: "split", id: newId(`${node.id}:split`, occupied),
      direction: position === "left" || position === "right" ? "horizontal" : "vertical", ratio: 0.5,
      first: leading ? pane : node, second: leading ? node : pane,
    };
  });
  return activatePane(next, pane.id);
}

/** Presets arrange every open pane, including inactive tabs; none are dropped. */
export function applyPreset(root: LayoutNode, preset: LayoutPreset, selectedPaneId?: string | null): LayoutNode {
  const panes = flattenPanes(root);
  const count = preset === "2x2" ? 4 : preset === "1:2:1" ? 3 : 2;
  const occupied = new Set(panes.map((pane) => pane.id));
  const groups: PaneNode[][] = Array.from({ length: count }, () => []);
  panes.forEach((pane, index) => groups[index % count].push(pane));
  const slots: LayoutNode[] = groups.map((group, index) => group.length === 1 ? group[0] : {
    type: "stack", kind: "stack", id: newId(`preset:group-${index}`, occupied), panes: group,
    ...(group.length ? { activePaneId: group.find((pane) => pane.id === selectedPaneId)?.id ?? group[0].id } : {}),
  });
  const split = (first: LayoutNode, second: LayoutNode, direction: SplitDirection, ratio = 0.5): SplitNode => ({
    type: "split", id: newId("preset:split", occupied), direction, ratio, first, second,
  });
  if (preset === "2x2") return split(split(slots[0], slots[1], "vertical"), split(slots[2], slots[3], "vertical"), "horizontal");
  if (preset === "1:2:1") return split(slots[0], split(slots[1], slots[2], "horizontal", 2 / 3), "horizontal", 0.25);
  return split(slots[0], slots[1], "horizontal");
}

export interface ProjectionOptions {
  minWidth?: number;
  minHeight?: number;
  selectedPaneId?: string | null;
  /** Remember a different active tab for each collapsed subtree. */
  activeByGroup?: Readonly<Record<string, string>>;
}
function preferredPane(node: LayoutNode): string | undefined {
  if (node.type === "pane") return node.id;
  if (node.type === "stack") return node.activePaneId ?? node.panes[0]?.id;
  return preferredPane(node.first) ?? preferredPane(node.second);
}

/**
 * Responsive collapse is a disposable VIEW projection. Retain and mutate the
 * canonical root. Growing a viewport restores current edits and ratios, never
 * a stale snapshot. No pane object or content is discarded.
 */
export function projectLayout(root: LayoutNode, width: number, height: number, options: ProjectionOptions = {}): LayoutNode {
  const minW = options.minWidth ?? MIN_PANE_WIDTH;
  const minH = options.minHeight ?? MIN_PANE_HEIGHT;
  const occupied = idsOf(root);
  const project = (node: LayoutNode, w: number, h: number): LayoutNode => {
    if (node.type !== "split") return node;
    const available = Math.max(0, (node.direction === "horizontal" ? w : h) - SPLIT_GAP);
    const ratio = normalizeRatio(node.ratio);
    const first = available * ratio;
    const second = available * (1 - ratio);
    const tooSmall = node.direction === "horizontal" ? first < minW || second < minW || h < minH : first < minH || second < minH || w < minW;
    if (tooSmall) {
      const panes = flattenPanes(node);
      const candidates = [options.selectedPaneId, options.activeByGroup?.[node.id], preferredPane(node)];
      const activePaneId = candidates.find((id) => !!id && panes.some((pane) => pane.id === id)) ?? panes[0]?.id;
      return { type: "stack", kind: "stack", id: newId(`${node.id}:view-tabs`, occupied), panes, activePaneId: activePaneId ?? undefined, collapsedFrom: node.id };
    }
    return {
      ...node,
      first: project(node.first, node.direction === "horizontal" ? first : w, node.direction === "vertical" ? first : h),
      second: project(node.second, node.direction === "horizontal" ? second : w, node.direction === "vertical" ? second : h),
    };
  };
  return project(root, Math.max(0, width), Math.max(0, height));
}

/** Read old persisted collapse snapshots once; new storage has no snapshots. */
export function restoreCollapsed(document: LayoutDocument, splitId?: string): LayoutDocument {
  if (!document.collapsed?.length) return document;
  let root = document.root;
  const remaining = [...document.collapsed];
  for (let pass = 0; pass < document.collapsed.length; pass += 1) {
    let restored = false;
    for (let index = remaining.length - 1; index >= 0; index -= 1) {
      const entry = remaining[index];
      if (splitId && entry.splitId !== splitId) continue;
      const current = findNode(root, entry.replacementId);
      if (current?.type !== "stack") continue;
      let recovered: LayoutNode = cloneNode(entry.node);
      const liveIds = new Set(current.panes.map((pane) => pane.id));
      const originalIds = new Set(flattenPanes(recovered).map((pane) => pane.id));
      for (const id of originalIds) if (!liveIds.has(id)) recovered = closePane(recovered, id);
      for (const pane of current.panes) recovered = originalIds.has(pane.id) ? updatePane(recovered, pane) : dockPane(recovered, pane, recovered.id, "center");
      if (current.activePaneId) recovered = activatePane(recovered, current.activePaneId);
      root = replaceNode(root, current.id, () => recovered);
      remaining.splice(index, 1);
      restored = true;
    }
    if (!restored) break;
  }
  return { version: document.version, root, ...(remaining.length ? { collapsed: remaining } : {}) };
}

export interface LayoutValidationResult { valid: boolean; errors: string[]; }
export function validateLayout(value: unknown): boolean;
export function validateLayout(value: unknown, detailed: true): LayoutValidationResult;
export function validateLayout(value: unknown, detailed = false): boolean | LayoutValidationResult {
  const errors: string[] = [];
  const ids = new Set<string>();
  let visited = 0;
  const visit = (candidate: unknown, path: string, depth: number) => {
    if (++visited > 4096 || depth > 128) { if (!errors.includes("layout exceeds safety limit")) errors.push("layout exceeds safety limit"); return; }
    if (!candidate || typeof candidate !== "object") { errors.push(`${path} must be an object`); return; }
    const node = candidate as Record<string, unknown>;
    if (typeof node.id !== "string" || !node.id) errors.push(`${path}.id must be a non-empty string`);
    else if (ids.has(node.id)) errors.push(`duplicate node id: ${node.id}`);
    else ids.add(node.id);
    if (node.type === "pane") {
      if (!LEAF_KINDS.includes(node.kind as LeafPaneKind)) errors.push(`${path}.kind is invalid`);
      if (node.title !== undefined && typeof node.title !== "string") errors.push(`${path}.title must be a string`);
      if (node.metadata !== undefined && (!node.metadata || typeof node.metadata !== "object" || Array.isArray(node.metadata))) errors.push(`${path}.metadata must be an object`);
    } else if (node.type === "stack") {
      if (node.kind !== "stack") errors.push(`${path}.kind must be stack`);
      if (!Array.isArray(node.panes)) errors.push(`${path}.panes must be an array`);
      else {
        for (const [index, pane] of node.panes.entries()) {
          if (!pane || typeof pane !== "object" || pane.type !== "pane") errors.push(`${path}.panes[${index}] must be a pane`);
          visit(pane, `${path}.panes[${index}]`, depth + 1);
        }
        if (node.activePaneId !== undefined && !node.panes.some((pane) => pane?.id === node.activePaneId)) errors.push(`${path}.activePaneId is missing from tabs`);
      }
    } else if (node.type === "split") {
      if (node.direction !== "horizontal" && node.direction !== "vertical") errors.push(`${path}.direction is invalid`);
      if (typeof node.ratio !== "number" || !Number.isFinite(node.ratio) || node.ratio <= 0 || node.ratio >= 1) errors.push(`${path}.ratio must be finite and between 0 and 1`);
      visit(node.first, `${path}.first`, depth + 1);
      visit(node.second, `${path}.second`, depth + 1);
    } else errors.push(`${path}.type is invalid`);
  };
  if (isDocument(value)) {
    if (!Number.isInteger(value.version) || value.version < 1) errors.push("version must be a positive integer");
    visit(value.root, "root", 0);
    if (value.collapsed !== undefined && !Array.isArray(value.collapsed)) errors.push("collapsed must be an array");
  } else visit(value, "root", 0);
  return detailed ? { valid: !errors.length, errors } : !errors.length;
}
export function isPaneNode(value: unknown): value is PaneNode {
  return !!value && typeof value === "object" && "type" in value && value.type === "pane" && validateLayout(value);
}

/**
 * The pane that stands for a node: the pane itself, a stack's active tab (or
 * its first), or the first pane under a split.
 */
export function representativePane(node: LayoutNode | undefined): PaneNode | undefined {
  if (!node) return undefined;
  if (node.type === "pane") return node;
  if (node.type === "stack") return node.panes.find(pane => pane.id === node.activePaneId) ?? node.panes[0];
  return flattenPanes(node)[0];
}

/**
 * Where a pane dropped into the middle of `id` goes. A split that is shown
 * collapsed into tabs (narrow screen) takes it beside the tab on show there,
 * in that tab's own container; anything else takes it as it is.
 */
export function centerDockTarget(root: LayoutNode, projected: LayoutNode, id: string): string {
  if (findNode(root, id)?.type !== "split") return id;
  const group = findNodes(projected, node => node.type === "stack" && node.collapsedFrom === id)[0];
  const activeId = group?.type === "stack" ? group.activePaneId : undefined;
  return activeId ? containerId(root, activeId) : id;
}
