// Soft alignment snapping for the infinite board — used by both terminal windows
// (Session.svelte) and board items (Board.svelte). Given a moving rect and the
// other rects on the canvas, find the nearest edge/center alignment within a
// pixel threshold and return the nudged position plus the guide lines to draw.
//
// "Soft" by design (Bo 2026-06-13: "ไม่ต้องฟิคขนาดนั้น") — it only pulls when an
// edge is already close, so free-form placement still works; it just makes
// lining things up with each other easy.

export interface SnapRect {
  /** Identifier so callers can exclude the rect currently being dragged. */
  id?: string;
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface SnapResult {
  /** Snapped top-left in the same (world) units as the input. */
  x: number;
  y: number;
  /** World x-coordinates of active vertical guide lines. */
  guidesV: number[];
  /** World y-coordinates of active horizontal guide lines. */
  guidesH: number[];
}

/**
 * @param left/top/width/height  the moving rect (world units)
 * @param others                 every other rect to align against (world units)
 * @param threshold              max gap (world units) at which a snap engages
 */
export function computeSnap(
  left: number,
  top: number,
  width: number,
  height: number,
  others: SnapRect[],
  threshold: number,
): SnapResult {
  const right = left + width;
  const bottom = top + height;
  const cx = left + width / 2;
  const cy = top + height / 2;

  // Scalars (not a nullable object) so TS control-flow doesn't choke on the
  // closures mutating an outer union — npm run check narrowed it to `never`.
  let hasX = false;
  let bestXDelta = 0;
  let bestXGuide = 0;
  let hasY = false;
  let bestYDelta = 0;
  let bestYGuide = 0;

  const considerX = (movingEdge: number, target: number) => {
    const d = target - movingEdge;
    if (Math.abs(d) <= threshold && (!hasX || Math.abs(d) < Math.abs(bestXDelta))) {
      hasX = true;
      bestXDelta = d;
      bestXGuide = target;
    }
  };
  const considerY = (movingEdge: number, target: number) => {
    const d = target - movingEdge;
    if (Math.abs(d) <= threshold && (!hasY || Math.abs(d) < Math.abs(bestYDelta))) {
      hasY = true;
      bestYDelta = d;
      bestYGuide = target;
    }
  };

  for (const o of others) {
    const oL = o.left;
    const oR = o.left + o.width;
    const oCx = o.left + o.width / 2;
    const oT = o.top;
    const oB = o.top + o.height;
    const oCy = o.top + o.height / 2;

    // Vertical guides: align left/center/right of the moving rect to the
    // left/center/right of each other rect.
    considerX(left, oL);
    considerX(left, oR);
    considerX(right, oR);
    considerX(right, oL);
    considerX(cx, oCx);

    // Horizontal guides: top/middle/bottom.
    considerY(top, oT);
    considerY(top, oB);
    considerY(bottom, oB);
    considerY(bottom, oT);
    considerY(cy, oCy);
  }

  const guidesV: number[] = [];
  const guidesH: number[] = [];
  let nx = left;
  let ny = top;
  if (hasX) {
    nx = left + bestXDelta;
    guidesV.push(bestXGuide);
  }
  if (hasY) {
    ny = top + bestYDelta;
    guidesH.push(bestYGuide);
  }
  return { x: nx, y: ny, guidesV, guidesH };
}
