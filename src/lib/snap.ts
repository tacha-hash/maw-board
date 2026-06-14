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
  | "center"
  | "firstThird"
  | "centerThird"
  | "lastThird"
  | "firstTwoThirds"
  | "centerTwoThirds"
  | "lastTwoThirds";

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

export type SnapEdge = "left" | "right" | "top" | "bottom";
export type SnapShortcutAction = SnapAction | "restore";

export interface SnapShortcutModifiers {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
}

export interface SnapShortcutKeyState {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey?: boolean;
  metaKey?: boolean;
}

// One place to change if Bo's OS grabs Ctrl+Alt+Arrow before the browser sees it.
export const DEFAULT_SNAP_SHORTCUT_MODIFIERS: SnapShortcutModifiers = {
  ctrl: true,
  alt: true,
  shift: false,
  meta: false,
};

export function isSnapAction(action: string): action is SnapAction {
  switch (action) {
    case "leftHalf":
    case "rightHalf":
    case "topHalf":
    case "bottomHalf":
    case "topLeft":
    case "topRight":
    case "bottomLeft":
    case "bottomRight":
    case "maximize":
    case "almostMaximize":
    case "maximizeHeight":
    case "center":
    case "firstThird":
    case "centerThird":
    case "lastThird":
    case "firstTwoThirds":
    case "centerTwoThirds":
    case "lastTwoThirds":
      return true;
    default:
      return false;
  }
}

function hasSnapShortcutModifiers(
  event: SnapShortcutKeyState,
  modifiers: SnapShortcutModifiers,
) {
  return (
    event.ctrlKey === modifiers.ctrl &&
    event.altKey === modifiers.alt &&
    !!event.shiftKey === modifiers.shift &&
    !!event.metaKey === modifiers.meta
  );
}

export function snapShortcutAction(
  event: SnapShortcutKeyState,
  modifiers = DEFAULT_SNAP_SHORTCUT_MODIFIERS,
): SnapShortcutAction | null {
  if (!hasSnapShortcutModifiers(event, modifiers)) return null;

  switch (event.key) {
    case "ArrowLeft":
      return "leftHalf";
    case "ArrowRight":
      return "rightHalf";
    case "ArrowUp":
      return "topHalf";
    case "ArrowDown":
      return "bottomHalf";
    case "u":
    case "U":
      return "topLeft";
    case "i":
    case "I":
      return "topRight";
    case "j":
    case "J":
      return "bottomLeft";
    case "k":
    case "K":
      return "bottomRight";
    case "f":
    case "F":
      return "maximize";
    case "c":
    case "C":
      return "center";
    case "1":
      return "firstThird";
    case "2":
      return "centerThird";
    case "3":
      return "lastThird";
    case "0":
    case "Backspace":
      return "restore";
    default:
      return null;
  }
}

function segmentStart(total: number, index: number, segments: number) {
  return Math.floor((total * index) / segments);
}

function segmentSize(total: number, index: number, span: number, segments: number) {
  return (
    segmentStart(total, index + span, segments) -
    segmentStart(total, index, segments)
  );
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
  const strip = coarse
    ? Math.min(180, Math.max(96, Math.floor(Math.min(viewport.w, viewport.h) * 0.22)))
    : Math.min(132, Math.max(56, Math.floor(Math.min(viewport.w, viewport.h) * 0.18)));

  const x = Math.max(0, Math.min(viewport.w, clientX));
  const y = Math.max(0, Math.min(viewport.h, clientY));
  const landscape = viewport.w >= viewport.h;

  if (x <= corner && y <= corner) return "topLeft";
  if (x >= viewport.w - corner && y <= corner) return "topRight";
  if (x <= corner && y >= viewport.h - corner) return "bottomLeft";
  if (x >= viewport.w - corner && y >= viewport.h - corner)
    return "bottomRight";

  if (y <= edge) return "maximize";
  if (x <= edge) {
    if (y <= strip) return "topHalf";
    if (y >= viewport.h - strip) return "bottomHalf";
    if (!landscape) {
      if (y < viewport.h / 3) return "firstThird";
      if (y > (viewport.h * 2) / 3) return "lastThird";
      return "centerThird";
    }
    return "leftHalf";
  }
  if (x >= viewport.w - edge) {
    if (y <= strip) return "topHalf";
    if (y >= viewport.h - strip) return "bottomHalf";
    if (!landscape) {
      if (y < viewport.h / 3) return "firstThird";
      if (y > (viewport.h * 2) / 3) return "lastThird";
      return "centerThird";
    }
    return "rightHalf";
  }
  if (y >= viewport.h - edge) {
    if (landscape) {
      if (x < viewport.w / 3) return "firstThird";
      if (x > (viewport.w * 2) / 3) return "lastThird";
      return "centerThird";
    }
    return x < viewport.w / 2 ? "leftHalf" : "rightHalf";
  }
  return null;
}

export function snapSharedEdges(action: SnapAction, view: ViewRect): SnapEdge[] {
  const landscape = view.w >= view.h;
  switch (action) {
    case "leftHalf":
      return ["right"];
    case "rightHalf":
      return ["left"];
    case "topHalf":
      return ["bottom"];
    case "bottomHalf":
      return ["top"];
    case "topLeft":
      return ["right", "bottom"];
    case "topRight":
      return ["left", "bottom"];
    case "bottomLeft":
      return ["right", "top"];
    case "bottomRight":
      return ["left", "top"];
    case "firstThird":
      return landscape ? ["right"] : ["bottom"];
    case "centerThird":
      return landscape ? ["left", "right"] : ["top", "bottom"];
    case "lastThird":
      return landscape ? ["left"] : ["top"];
    case "firstTwoThirds":
      return landscape ? ["right"] : ["bottom"];
    case "centerTwoThirds":
      return landscape ? ["left", "right"] : ["top", "bottom"];
    case "lastTwoThirds":
      return landscape ? ["left"] : ["top"];
    case "almostMaximize":
    case "center":
    case "maximize":
    case "maximizeHeight":
      return [];
  }
}

export function applySnapGap(
  rect: ViewRect,
  gap: number,
  sharedEdges: SnapEdge[] = [],
): ViewRect {
  if (!Number.isFinite(gap) || gap <= 0) return rect;

  const inset = Math.max(0, gap);
  const shared = new Set(sharedEdges);
  const left = shared.has("left") ? inset / 2 : inset;
  const right = shared.has("right") ? inset / 2 : inset;
  const top = shared.has("top") ? inset / 2 : inset;
  const bottom = shared.has("bottom") ? inset / 2 : inset;

  return {
    x: rect.x + left,
    y: rect.y + top,
    w: Math.max(1, rect.w - left - right),
    h: Math.max(1, rect.h - top - bottom),
  };
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
  const landscape = view.w >= view.h;
  const twoThirds = landscape
    ? segmentSize(view.w, 0, 2, 3)
    : segmentSize(view.h, 0, 2, 3);
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
    case "firstThird":
      return landscape
        ? {
            x: view.x + segmentStart(view.w, 0, 3),
            y: view.y,
            w: segmentSize(view.w, 0, 1, 3),
            h: view.h,
          }
        : {
            x: view.x,
            y: view.y + segmentStart(view.h, 0, 3),
            w: view.w,
            h: segmentSize(view.h, 0, 1, 3),
          };
    case "centerThird":
      return landscape
        ? {
            x: view.x + segmentStart(view.w, 1, 3),
            y: view.y,
            w: segmentSize(view.w, 1, 1, 3),
            h: view.h,
          }
        : {
            x: view.x,
            y: view.y + segmentStart(view.h, 1, 3),
            w: view.w,
            h: segmentSize(view.h, 1, 1, 3),
          };
    case "lastThird":
      return landscape
        ? {
            x: view.x + segmentStart(view.w, 2, 3),
            y: view.y,
            w: segmentSize(view.w, 2, 1, 3),
            h: view.h,
          }
        : {
            x: view.x,
            y: view.y + segmentStart(view.h, 2, 3),
            w: view.w,
            h: segmentSize(view.h, 2, 1, 3),
          };
    case "firstTwoThirds":
      return landscape
        ? { x: view.x, y: view.y, w: twoThirds, h: view.h }
        : { x: view.x, y: view.y, w: view.w, h: twoThirds };
    case "centerTwoThirds":
      return landscape
        ? {
            x: view.x + Math.floor((view.w - twoThirds) / 2),
            y: view.y,
            w: twoThirds,
            h: view.h,
          }
        : {
            x: view.x,
            y: view.y + Math.floor((view.h - twoThirds) / 2),
            w: view.w,
            h: twoThirds,
          };
    case "lastTwoThirds":
      return landscape
        ? {
            x: view.x + segmentStart(view.w, 1, 3),
            y: view.y,
            w: segmentSize(view.w, 1, 2, 3),
            h: view.h,
          }
        : {
            x: view.x,
            y: view.y + segmentStart(view.h, 1, 3),
            w: view.w,
            h: segmentSize(view.h, 1, 2, 3),
          };
  }
}
