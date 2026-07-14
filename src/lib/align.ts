// VR4 group menu — align + distribute for a multi-selection of board items.
// Pure geometry (no DOM, no Svelte) so it's trivially testable and the group
// menu just maps the returned {id,x,y} list onto handleBoardMove.
//
// All coordinates are world units (same basis as BoardItem.x/y/w/h). Each
// function returns ONLY the items whose position actually changes, so a
// no-op align sends no board updates.
import type { BoardItem } from "./protocol";

export type AlignEdge = "left" | "center-h" | "right" | "top" | "middle-v" | "bottom";
export type DistributeAxis = "h" | "v";

export type Placement = { id: string; x: number; y: number };

/** Align every item to a shared edge/centre of the selection's combined
 *  bounding box. Needs >= 2 items to mean anything. */
export function alignItems(items: BoardItem[], edge: AlignEdge): Placement[] {
  if (items.length < 2) return [];
  const minX = Math.min(...items.map((i) => i.x));
  const maxX = Math.max(...items.map((i) => i.x + i.w));
  const minY = Math.min(...items.map((i) => i.y));
  const maxY = Math.max(...items.map((i) => i.y + i.h));
  const cx = (minX + maxX) / 2;
  const cy = (minY + maxY) / 2;
  const out: Placement[] = [];
  for (const it of items) {
    let x = it.x;
    let y = it.y;
    switch (edge) {
      case "left":
        x = minX;
        break;
      case "right":
        x = maxX - it.w;
        break;
      case "center-h":
        x = Math.round(cx - it.w / 2);
        break;
      case "top":
        y = minY;
        break;
      case "bottom":
        y = maxY - it.h;
        break;
      case "middle-v":
        y = Math.round(cy - it.h / 2);
        break;
    }
    if (x !== it.x || y !== it.y) out.push({ id: it.id, x, y });
  }
  return out;
}

/** Distribute items so the GAPS between adjacent items are equal along the
 *  axis. The two extreme items stay put (anchors); only the middle ones move,
 *  so it needs >= 3 items. */
export function distributeItems(items: BoardItem[], axis: DistributeAxis): Placement[] {
  const n = items.length;
  if (n < 3) return [];
  const lead = (i: BoardItem) => (axis === "h" ? i.x : i.y);
  const size = (i: BoardItem) => (axis === "h" ? i.w : i.h);
  const sorted = [...items].sort((a, b) => lead(a) - lead(b));
  const first = sorted[0];
  const last = sorted[n - 1];
  const span = lead(last) + size(last) - lead(first); // outer-edge to outer-edge
  const totalSize = sorted.reduce((s, i) => s + size(i), 0);
  const gap = (span - totalSize) / (n - 1);
  const out: Placement[] = [];
  let cursor = lead(first) + size(first) + gap;
  for (let k = 1; k < n - 1; k++) {
    const it = sorted[k];
    const pos = Math.round(cursor);
    if (axis === "h") {
      if (pos !== it.x) out.push({ id: it.id, x: pos, y: it.y });
    } else {
      if (pos !== it.y) out.push({ id: it.id, x: it.x, y: pos });
    }
    cursor += size(it) + gap;
  }
  return out;
}
