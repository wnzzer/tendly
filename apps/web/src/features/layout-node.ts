import type { DockPosition } from "../layout/layout-engine";
import { normalizeRatio, SPLIT_GAP } from "../layout/layout-engine";
import type { PaneNode, SplitDirection } from "@tendly/protocol/layout";

/**
 * The geometry and keyboard rules of one node of the canvas: where a dragged
 * pane would dock, the grid a split lays out, how the keys move a split's
 * divider and step through tabs, and which tabs a bulk close takes.
 */

/**
 * Where a pane dropped at (`x`, `y`) inside a pane `width` by `height` docks.
 * Over the tab strip (the top `header` pixels) it joins the tabs. Below it,
 * the outer third of the content beside each edge splits that way, the
 * nearest edge winning in a corner, and only the middle joins the tabs: a
 * split has to be easy to hit, since the tab strip already takes a tab.
 */
export function dockPosition(x: number, y: number, width: number, height: number, header = 0): DockPosition {
  if (y < header) return "center";
  const top = y - header, body = height - header;
  const marginX = width * DOCK_EDGE, marginY = body * DOCK_EDGE;
  const candidates: [DockPosition, number][] = [["left", x / marginX], ["right", (width - x) / marginX], ["top", top / marginY], ["bottom", (body - top) / marginY]];
  const nearest = candidates.sort((a, b) => a[1] - b[1])[0]!;
  return nearest[1] < 1 ? nearest[0] : "center";
}
const DOCK_EDGE = 1 / 3;

/** The CSS grid a split lays its two sides out on, the divider between them. */
export function splitGrid(direction: SplitDirection, ratio: number): Record<string, string> {
  const share = normalizeRatio(ratio);
  const tracks = `minmax(0, ${share}fr) ${SPLIT_GAP}px minmax(0, ${1 - share}fr)`;
  return direction === "horizontal" ? { gridTemplateColumns: tracks, gridTemplateRows: "minmax(0, 1fr)" } : { gridTemplateRows: tracks, gridTemplateColumns: "minmax(0, 1fr)" };
}

/**
 * A split divider's new ratio for a key, or `undefined` for a key it ignores:
 * the arrows along the split nudge it (further with Shift), Home centres it.
 */
export function keyedRatio(key: string, direction: SplitDirection, ratio: number, shift: boolean): number | undefined {
  if (key === "Home") return 0.5;
  const horizontal = direction === "horizontal";
  const decrease = key === (horizontal ? "ArrowLeft" : "ArrowUp");
  const increase = key === (horizontal ? "ArrowRight" : "ArrowDown");
  if (!decrease && !increase) return undefined;
  return normalizeRatio(ratio + (increase ? 1 : -1) * (shift ? 0.1 : 0.025));
}

/** The tab a key moves to, wrapping at the ends, or `undefined` for a key that does not move. */
export function keyedTab(key: string, index: number, count: number): number | undefined {
  if (!count) return undefined;
  if (key === "Home") return 0;
  if (key === "End") return count - 1;
  if (key === "ArrowRight") return (index + 1) % count;
  if (key === "ArrowLeft") return (index - 1 + count) % count;
  return undefined;
}

/** The tabs beside one: those to its left and right and all the others, for closing in bulk. */
export function tabsAround(tabs: readonly PaneNode[], id: string): { left: PaneNode[]; right: PaneNode[]; others: PaneNode[] } {
  const index = tabs.findIndex(pane => pane.id === id);
  return {
    left: index < 0 ? [] : tabs.slice(0, index),
    right: index < 0 ? [] : tabs.slice(index + 1),
    others: tabs.filter(pane => pane.id !== id),
  };
}
