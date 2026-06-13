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

// ── Rectangle-style snap layouts (Bo 2026-06-13) ────────────────────────────
// Snap a window into a fraction of the *visible viewport* (the Rectangle app
// model, mapped to the board's world coords by Session.svelte). Pure geometry:
// given the viewport world-rect, return the target world-rect for an action.
// Board y grows downward, so "top" = smaller y (no macOS flipped-frame naming).
export type SnapAction =
  | "leftHalf"
  | "rightHalf"
  | "topHalf"
  | "bottomHalf"
  | "topLeft"
  | "topRight"
  | "bottomLeft"
  | "bottomRight"
  | "maximize"
  | "almostMaximize"
  | "maximizeHeight"
  | "center";

export interface ViewRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface ViewportPx {
  w: number;
  h: number;
}

export interface EdgeSnapOptions {
  coarse?: boolean;
}

/**
 * Detect Rectangle-style drag-to-edge snap actions from viewport/client pixels.
 * Activation is intentionally screen-space: touch users need a wider edge zone,
 * and the trigger should not change with board zoom.
 */
export function detectEdgeSnapAction(
  clientX: number,
  clientY: number,
  viewport: ViewportPx,
  options: EdgeSnapOptions = {},
): SnapAction | null {
  if (viewport.w <= 0 || viewport.h <= 0) return null;

  const coarse = options.coarse ?? false;
  const edge = coarse ? 32 : 14;
  const corner = coarse ? 84 : 48;

  const x = Math.max(0, Math.min(viewport.w, clientX));
  const y = Math.max(0, Math.min(viewport.h, clientY));

  if (x <= corner && y <= corner) return "topLeft";
  if (x >= viewport.w - corner && y <= corner) return "topRight";
  if (x <= corner && y >= viewport.h - corner) return "bottomLeft";
  if (x >= viewport.w - corner && y >= viewport.h - corner)
    return "bottomRight";

  if (y <= edge) return "maximize";
  if (x <= edge) return "leftHalf";
  if (x >= viewport.w - edge) return "rightHalf";
  if (y >= viewport.h - edge) return "bottomHalf";
  return null;
}

/**
 * @param action  which snap layout to apply
 * @param view    the visible viewport as a world rect
 * @param current the window's current world rect — only maximizeHeight/center
 *                read it (they preserve one axis / the current size)
 */
export function computeSnapTarget(
  action: SnapAction,
  view: ViewRect,
  current?: ViewRect,
): ViewRect {
  const hw = Math.floor(view.w / 2);
  const hh = Math.floor(view.h / 2);
  const rightX = view.x + view.w - hw; // right column starts at maxX - halfWidth
  const bottomY = view.y + view.h - hh;
  switch (action) {
    case "leftHalf":
      return { x: view.x, y: view.y, w: hw, h: view.h };
    case "rightHalf":
      return { x: rightX, y: view.y, w: hw, h: view.h };
    case "topHalf":
      return { x: view.x, y: view.y, w: view.w, h: hh };
    case "bottomHalf":
      return { x: view.x, y: bottomY, w: view.w, h: hh };
    case "topLeft":
      return { x: view.x, y: view.y, w: hw, h: hh };
    case "topRight":
      return { x: rightX, y: view.y, w: hw, h: hh };
    case "bottomLeft":
      return { x: view.x, y: bottomY, w: hw, h: hh };
    case "bottomRight":
      return { x: rightX, y: bottomY, w: hw, h: hh };
    case "maximize":
      return { x: view.x, y: view.y, w: view.w, h: view.h };
    case "almostMaximize": {
      const w = Math.floor(view.w * 0.9);
      const h = Math.floor(view.h * 0.9);
      return {
        x: view.x + Math.floor((view.w - w) / 2),
        y: view.y + Math.floor((view.h - h) / 2),
        w,
        h,
      };
    }
    case "maximizeHeight": {
      const c = current ?? view; // keep current x/width, fill viewport height
      return { x: c.x, y: view.y, w: c.w, h: view.h };
    }
    case "center": {
      const c = current ?? view; // keep current size (clamped), recenter
      const w = Math.min(c.w, view.w);
      const h = Math.min(c.h, view.h);
      return {
        x: view.x + Math.floor((view.w - w) / 2),
        y: view.y + Math.floor((view.h - h) / 2),
        w,
        h,
      };
    }
  }
}
