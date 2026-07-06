<script lang="ts">
  import { createEventDispatcher, onDestroy } from "svelte";
  import { fade } from "svelte/transition";
  import { XIcon, DownloadIcon } from "svelte-feather-icons";

  import { slide } from "../action/slide";
  import { computeSnap, type SnapRect } from "../snap";
  import type { BoardItem } from "../protocol";

  /** Board items (images + screen-share placeholder tiles). */
  export let items: BoardItem[];
  /** Live screen-share frame object URLs, keyed by item id. */
  export let streamSrcs: Record<string, string>;
  export let center: number[];
  export let zoom: number;
  export let hasWriteAccess: boolean | undefined;

  // Fabric offset CSS, shared with the terminal layer so tiles live in the same
  // infinite canvas and pan/zoom together.
  export let offsetLeftCss: string;
  export let offsetTopCss: string;
  export let offsetTransformOriginCss: string;

  /** Maps a pointer event to world-grid coordinates (same basis as terminals). */
  export let normalizePosition: (event: MouseEvent) => [number, number];
  /** Other rects (terminals + items) to soft-snap against, in world units. */
  export let snapTargets: SnapRect[] = [];
  /** Guide lines from a terminal drag (Session), rendered on the same fabric. */
  export let extraGuidesV: number[] = [];
  export let extraGuidesH: number[] = [];

  // Screen-space snap distance; converted to world units (÷ zoom) so the pull
  // feels the same regardless of how far the board is zoomed.
  const SNAP_PX = 8;
  // Active guide lines (world coords) while dragging.
  let guidesV: number[] = [];
  let guidesH: number[] = [];

  const dispatch = createEventDispatcher<{
    move: { id: string; x: number; y: number };
    resize: { id: string; w: number; h: number };
    delete: string;
    edit: { id: string; text: string };
    jobEdit: { id: string; prompt?: string; model?: string };
    jobGenerate: string;
    jobRetry: string;
  }>();

  // ── Image-gen job node — dataUrl is a JSON payload, not raw text (see
  // docs/round2-frontend-design.md). Parsed defensively like every other
  // JSON-payload board item (link/lock precedent) since a malformed/partial
  // dataUrl must never crash the render loop.
  //
  // Field names/values here MUST match tools/board-bridge.ts exactly (it's
  // the live consumer, not a spec we control): `state`, not `status`; a
  // *draft* job must carry an explicit `state: "draft"` — the bridge treats
  // a MISSING state as immediately processable ("pending"-equivalent), so
  // omitting it while the user is still typing would make the bridge start
  // generating on every keystroke.
  type JobState = "draft" | "pending" | "running" | "done" | "error";
  type JobPayload = {
    prompt: string;
    model: string;
    state: JobState;
    error?: string;
  };
  // First entry matches the bridge's own fallback default (runKieJob's `m`).
  // Must match board-bridge.ts's MODEL_ALIASES keys exactly (confirmed by
  // reading its source, commit 0f7f6d3) — anything else passes through to
  // Kie unaliased and 422s (found live: "seedream" alone isn't a valid Kie
  // model id, needs the "bytedance/seedream" the bridge maps it to).
  const JOB_MODELS = ["nano-banana", "flux", "seedream", "gpt-image"];
  function parseJob(dataUrl: string): JobPayload {
    try {
      const parsed = JSON.parse(dataUrl);
      const state: JobState = ["draft", "pending", "running", "done", "error"].includes(
        parsed.state,
      )
        ? parsed.state
        : "draft";
      return {
        prompt: typeof parsed.prompt === "string" ? parsed.prompt : "",
        model: typeof parsed.model === "string" ? parsed.model : JOB_MODELS[0],
        state,
        error: typeof parsed.error === "string" ? parsed.error : undefined,
      };
    } catch {
      return { prompt: "", model: JOB_MODELS[0], state: "draft" };
    }
  }

  // Resize state — drag the bottom-right handle to stretch a tile (like a window).
  const MIN_W = 120;
  const MIN_H = 80;
  let resizeId: string | null = null;
  let resizePointerId: number | null = null;
  let resizeStartW = 0;
  let resizeStartH = 0;
  let resizeStartWorld = [0, 0];
  let resizePending = false;
  let latestResizeEvent: PointerEvent | null = null;

  function startResize(event: PointerEvent, item: BoardItem) {
    if (hasWriteAccess === false) return;
    event.preventDefault();
    event.stopPropagation();
    resizeId = item.id;
    resizePointerId = event.pointerId ?? null;
    resizeStartW = item.w || MIN_W;
    resizeStartH = item.h || MIN_H;
    resizeStartWorld = normalizePosition(event);
    window.addEventListener("pointermove", onResize);
    window.addEventListener("pointerup", endResize);
    window.addEventListener("pointercancel", endResize);
  }

  function currentResize(event: PointerEvent) {
    const [wx, wy] = normalizePosition(event);
    const w = Math.max(MIN_W, Math.round(resizeStartW + (wx - resizeStartWorld[0])));
    const h = Math.max(MIN_H, Math.round(resizeStartH + (wy - resizeStartWorld[1])));
    return { w, h };
  }

  function onResize(event: PointerEvent) {
    if (resizeId === null || event.pointerId !== resizePointerId) return;
    latestResizeEvent = event; // use the freshest sample inside the frame
    if (!resizePending) {
      resizePending = true;
      requestAnimationFrame(() => {
        resizePending = false;
        if (resizeId !== null && latestResizeEvent) {
          const { w, h } = currentResize(latestResizeEvent);
          dispatch("resize", { id: resizeId, w, h });
        }
      });
    }
  }

  function endResize(event: PointerEvent) {
    if (event.pointerId !== resizePointerId) return;
    if (resizeId !== null) {
      const { w, h } = currentResize(event);
      dispatch("resize", { id: resizeId, w, h });
    }
    resizeId = null;
    resizePointerId = null;
    latestResizeEvent = null;
    window.removeEventListener("pointermove", onResize);
    window.removeEventListener("pointerup", endResize);
    window.removeEventListener("pointercancel", endResize);
  }

  // Drag state. While dragging, the dragged tile renders at `dragPos` and sends
  // BoardMove on a requestAnimationFrame cadence (contract v2: client throttle).
  const LONG_PRESS_MS = 180;

  let dragId: string | null = null;
  let dragPointerId: number | null = null;
  let dragOffset = [0, 0];
  let dragPos = [0, 0];
  let rafPending = false;

  // Long-press state for mobile: hold ~400ms before drag activates.
  let pressTimer: ReturnType<typeof setTimeout> | null = null;
  let pressItem: BoardItem | null = null;
  let pressEvent: PointerEvent | null = null;
  let longPressActive = false;

  function onPointerDown(event: PointerEvent, item: BoardItem) {
    if (hasWriteAccess === false) return;
    // Keep the gesture on the item — don't let the canvas pan-handler also
    // claim it (caused touch drags to fight board panning).
    event.stopPropagation();
    // Desktop (mouse): start drag immediately.
    if (event.pointerType === "mouse") {
      startDrag(event, item);
      return;
    }
    // Touch: wait for long-press before allowing drag.
    if (pressTimer) {
      clearTimeout(pressTimer);
      window.removeEventListener("pointermove", onPressMove);
      window.removeEventListener("pointerup", cancelPress);
      window.removeEventListener("pointercancel", cancelPress);
    }
    pressItem = item;
    pressEvent = event;
    longPressActive = false;
    pressTimer = setTimeout(() => {
      if (pressItem && pressEvent) {
        // Hand off cleanly to the drag listeners — drop the press-phase ones so
        // a later pointerup doesn't fire both cancelPress and endDrag.
        window.removeEventListener("pointermove", onPressMove);
        window.removeEventListener("pointerup", cancelPress);
        window.removeEventListener("pointercancel", cancelPress);
        longPressActive = true;
        startDrag(pressEvent, pressItem);
      }
    }, LONG_PRESS_MS);
    window.addEventListener("pointermove", onPressMove);
    window.addEventListener("pointerup", cancelPress);
    window.addEventListener("pointercancel", cancelPress);
  }

  function onPressMove(event: PointerEvent) {
    if (longPressActive) {
      // Drag is active — the dedicated onMove listener handles movement.
      // Calling onMove here too double-processed every pointermove.
      return;
    }
    if (pressEvent && event.pointerId !== pressEvent.pointerId) return;
    // If finger moves too far before long-press fires, cancel.
    if (pressEvent) {
      const dx = event.clientX - pressEvent.clientX;
      const dy = event.clientY - pressEvent.clientY;
      if (dx * dx + dy * dy > 100) cancelPress(event);
    }
  }

  function cancelPress(event?: PointerEvent) {
    if (event && pressEvent && event.pointerId !== pressEvent.pointerId) return;
    if (pressTimer) clearTimeout(pressTimer);
    pressTimer = null;
    pressItem = null;
    pressEvent = null;
    if (longPressActive) {
      if (event) {
        endDrag(event);
      } else {
        dragId = null;
        dragPointerId = null;
        longPressActive = false;
        guidesV = [];
        guidesH = [];
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", endDrag);
        window.removeEventListener("pointercancel", endDrag);
      }
    }
    window.removeEventListener("pointermove", onPressMove);
    window.removeEventListener("pointerup", cancelPress);
    window.removeEventListener("pointercancel", cancelPress);
  }

  function startDrag(event: PointerEvent, item: BoardItem) {
    if (hasWriteAccess === false) return;
    event.preventDefault();
    const [wx, wy] = normalizePosition(event);
    dragId = item.id;
    dragPointerId = event.pointerId ?? null;
    dragOffset = [wx - item.x, wy - item.y];
    dragPos = [item.x, item.y];
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", endDrag);
    window.addEventListener("pointercancel", endDrag);
  }

  function onMove(event: PointerEvent) {
    if (dragId === null || event.pointerId !== dragPointerId) return;
    const [wx, wy] = normalizePosition(event);
    let nx = Math.round(wx - dragOffset[0]);
    let ny = Math.round(wy - dragOffset[1]);

    // Soft-snap this item's edges/center to other items + terminals, and light
    // up the guide lines we aligned to.
    const item = items.find((it) => it.id === dragId);
    if (item && snapTargets.length) {
      const others = snapTargets.filter((t) => t.id !== dragId);
      const r = computeSnap(nx, ny, item.w, item.h, others, SNAP_PX / zoom);
      nx = r.x;
      ny = r.y;
      guidesV = r.guidesV;
      guidesH = r.guidesH;
    }
    dragPos = [nx, ny];

    if (!rafPending) {
      rafPending = true;
      requestAnimationFrame(() => {
        rafPending = false;
        if (dragId !== null) {
          dispatch("move", { id: dragId, x: dragPos[0], y: dragPos[1] });
        }
      });
    }
  }

  function endDrag(event: PointerEvent) {
    if (event.pointerId !== dragPointerId) return;
    if (dragId !== null) {
      dispatch("move", { id: dragId, x: dragPos[0], y: dragPos[1] });
    }
    dragId = null;
    dragPointerId = null;
    longPressActive = false;
    guidesV = [];
    guidesH = [];
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", endDrag);
    window.removeEventListener("pointercancel", endDrag);
  }

  // Real download. A bare `<a download href={dataUrl}>` silently fails on mobile
  // (iOS Safari ignores `download` for data: URLs; Android blocks big ones), so
  // re-fetch into a Blob, hand out an object URL with a proper filename + ext,
  // and fall back to opening the raw URL (long-press-to-save) if that throws.
  async function downloadItem(item: BoardItem) {
    const ext = (mime: string) =>
      ({ "image/png": "png", "image/jpeg": "jpg", "image/webp": "webp", "image/gif": "gif", "video/mp4": "mp4", "video/webm": "webm" })[
        mime
      ] ?? (item.kind === "video" ? "mp4" : "png");
    try {
      const res = await fetch(item.dataUrl);
      if (!res.ok) throw new Error(`fetch ${res.status}`);
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${item.kind}-${item.id.slice(0, 6)}.${ext(blob.type)}`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      // Mobile share/save sheets can be slow — revoke late, not after 2s.
      setTimeout(() => URL.revokeObjectURL(url), 60_000);
    } catch {
      window.open(item.dataUrl, "_blank");
    }
  }

  onDestroy(() => {
    if (pressTimer) clearTimeout(pressTimer);
    window.removeEventListener("pointermove", onPressMove);
    window.removeEventListener("pointerup", cancelPress);
    window.removeEventListener("pointercancel", cancelPress);
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", endDrag);
    window.removeEventListener("pointercancel", endDrag);
    window.removeEventListener("pointermove", onResize);
    window.removeEventListener("pointerup", endResize);
    window.removeEventListener("pointercancel", endResize);
  });
</script>

{#each items.filter((it) => it.kind !== "doc" && it.kind !== "lock" && it.kind !== "label" && it.kind !== "link") as item (item.id)}
  {@const x = item.id === dragId ? dragPos[0] : item.x}
  {@const y = item.id === dragId ? dragPos[1] : item.y}
  <div
    class="absolute"
    style:left={offsetLeftCss}
    style:top={offsetTopCss}
    style:transform-origin={offsetTransformOriginCss}
    transition:fade|local
    use:slide={{ x, y, center, zoom, immediate: item.id === dragId }}
  >
    <div
      class="board-item"
      class:is-stream={item.kind === "stream"}
      class:is-note={item.kind === "note"}
      class:is-job={item.kind === "job"}
      style:width="{item.w}px"
      on:pointerdown={(event) => onPointerDown(event, item)}
    >
      {#if item.kind === "job"}
        {@const job = parseJob(item.dataUrl)}
        {@const editable = job.state === "draft"}
        <div class="job-node" style:height="{item.h}px">
          <div class="job-head">
            <span class="job-title">🎨 Image Gen</span>
            <select
              class="job-model"
              value={job.model}
              disabled={hasWriteAccess === false || !editable}
              on:pointerdown={(event) => event.stopPropagation()}
              on:change={(event) =>
                dispatch("jobEdit", {
                  id: item.id,
                  model: event.currentTarget.value,
                })}
            >
              {#each JOB_MODELS as m}
                <option value={m}>{m}</option>
              {/each}
            </select>
          </div>
          <textarea
            class="job-prompt"
            placeholder="Describe the image…"
            value={job.prompt}
            readonly={hasWriteAccess === false || !editable}
            on:pointerdown={(event) => event.stopPropagation()}
            on:input={(event) =>
              dispatch("jobEdit", {
                id: item.id,
                prompt: event.currentTarget.value,
              })}
          />
          <div class="job-footer">
            {#if job.state === "pending"}
              <span class="job-status">⏳ Queued — waiting for bridge…</span>
            {:else if job.state === "running"}
              <span class="job-status">🎨 Generating…</span>
            {:else if job.state === "error"}
              <span class="job-status job-status-error"
                >⚠ {job.error ?? "Generation failed"}</span
              >
              <button
                class="job-btn"
                on:pointerdown={(event) => event.stopPropagation()}
                on:click={() => dispatch("jobRetry", item.id)}
              >
                Retry
              </button>
            {:else if job.state === "done"}
              <span class="job-status">✅ Done — see image on the board</span>
              <button
                class="job-btn"
                on:pointerdown={(event) => event.stopPropagation()}
                on:click={() => dispatch("jobRetry", item.id)}
              >
                Generate again
              </button>
            {:else}
              <button
                class="job-btn job-btn-primary"
                disabled={hasWriteAccess === false || !job.prompt.trim()}
                on:pointerdown={(event) => event.stopPropagation()}
                on:click={() => dispatch("jobGenerate", item.id)}
              >
                ▶ Generate
              </button>
            {/if}
          </div>
        </div>
      {:else if item.kind === "note"}
        <textarea
          class="note-text"
          style:height="{item.h}px"
          placeholder="Type a note…"
          value={item.dataUrl}
          readonly={hasWriteAccess === false}
          on:pointerdown={(event) => event.stopPropagation()}
          on:input={(event) =>
            dispatch("edit", {
              id: item.id,
              text: event.currentTarget.value,
            })}
        />
      {:else if item.kind === "video"}
        <!-- svelte-ignore a11y-media-has-caption -->
        <video
          src={item.dataUrl}
          style:width="{item.w}px"
          controls
          playsinline
          on:pointerdown={(event) => event.stopPropagation()}
        />
      {:else}
        <img
          src={streamSrcs[item.id] ?? item.dataUrl}
          alt={item.kind === "stream" ? "screen share" : "shared image"}
          draggable="false"
        />
      {/if}

      {#if item.kind === "stream"}
        <div class="live-tag">● LIVE</div>
      {/if}

      {#if item.kind === "video" || item.kind === "image"}
        <button
          class="download"
          title="Download {item.kind}"
          on:pointerdown={(event) => event.stopPropagation()}
          on:click={() => downloadItem(item)}
        >
          <DownloadIcon size="14" />
        </button>
      {/if}

      {#if hasWriteAccess !== false}
        <button
          class="drag-grip"
          title="Drag to move"
          on:pointerdown={(event) => {
            event.stopPropagation();
            startDrag(event, item);
          }}
        >
          ⠿
        </button>
        <button
          class="delete"
          title="Remove"
          on:pointerdown={(event) => event.stopPropagation()}
          on:click={() => dispatch("delete", item.id)}
        >
          <XIcon size="14" />
        </button>
        <!-- Resize handle: drag to stretch the tile like a window. -->
        <div
          class="resize-handle"
          class:resizing={resizeId === item.id}
          title="Drag to resize"
          on:pointerdown={(event) => startResize(event, item)}
        />
      {/if}
    </div>
  </div>
{/each}

<!-- Alignment guides: thin lines through the world coords we snapped to. They
     ride the same canvas fabric (slide) as the items so they track pan/zoom. -->
{#each [...guidesV, ...extraGuidesV] as gx}
  <div
    class="absolute pointer-events-none"
    style:left={offsetLeftCss}
    style:top={offsetTopCss}
    style:transform-origin={offsetTransformOriginCss}
    use:slide={{ x: gx, y: center[1], center, zoom, immediate: true }}
  >
    <div class="guide guide-v" />
  </div>
{/each}
{#each [...guidesH, ...extraGuidesH] as gy}
  <div
    class="absolute pointer-events-none"
    style:left={offsetLeftCss}
    style:top={offsetTopCss}
    style:transform-origin={offsetTransformOriginCss}
    use:slide={{ x: center[0], y: gy, center, zoom, immediate: true }}
  >
    <div class="guide guide-h" />
  </div>
{/each}

<style lang="postcss">
  .board-item {
    @apply relative rounded-lg overflow-hidden bg-zinc-900 shadow-lg cursor-move select-none;
    @apply ring-1 ring-zinc-700 transition-transform duration-150;
  }

  /* Snap guide lines — span well past the viewport so they read as full-length
     rules; width/height in world px (the slide transform scales them by zoom). */
  .guide {
    @apply absolute pointer-events-none;
    background: theme("colors.indigo.400");
  }
  .guide-v {
    width: 1px;
    height: 6000px;
    top: -3000px;
    left: 0;
  }
  .guide-h {
    height: 1px;
    width: 6000px;
    left: -3000px;
    top: 0;
  }

  .board-item:active {
    @apply scale-[1.02];
  }

  .board-item.is-stream {
    @apply ring-2 ring-red-500/70;
  }

  .board-item.is-note {
    @apply bg-amber-200 ring-amber-300/60 shadow-amber-900/30;
  }

  .note-text {
    @apply block w-full p-3 bg-transparent resize-none outline-none border-0;
    @apply text-sm text-amber-950 placeholder-amber-700/50 font-medium leading-snug;
    cursor: text;
  }

  .board-item.is-job {
    @apply ring-indigo-400/60 shadow-indigo-900/30;
  }

  .job-node {
    @apply flex flex-col gap-2 p-2.5 bg-zinc-900 cursor-default;
  }

  .job-head {
    @apply flex items-center justify-between gap-2;
  }

  .job-title {
    @apply text-xs font-semibold text-zinc-300;
  }

  .job-model {
    @apply text-[11px] px-1.5 py-0.5 rounded bg-zinc-800 text-zinc-300 outline-none;
    @apply ring-1 ring-zinc-700 cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed;
  }

  .job-prompt {
    @apply flex-1 min-h-[100px] p-2 rounded-md bg-zinc-800/80 resize-none outline-none border-0;
    @apply text-sm text-zinc-100 placeholder-zinc-500 leading-snug ring-1 ring-zinc-700;
    @apply focus:ring-indigo-500;
    cursor: text;
  }

  .job-footer {
    @apply flex items-center gap-2 min-h-[28px];
  }

  .job-status {
    @apply text-xs text-zinc-400 flex-1 truncate;
  }

  .job-status-error {
    @apply text-red-400;
  }

  .job-btn {
    @apply text-xs px-2.5 py-1 rounded-md bg-zinc-700/80 text-zinc-200 hover:bg-zinc-600;
  }

  .job-btn-primary {
    @apply bg-indigo-600 text-white hover:bg-indigo-500 ml-auto;
    @apply disabled:opacity-40 disabled:hover:bg-indigo-600 disabled:cursor-not-allowed;
  }

  .board-item img {
    @apply block w-full h-auto pointer-events-none;
  }

  .board-item video {
    @apply block w-full h-auto bg-black;
  }

  .live-tag {
    @apply absolute top-1.5 left-1.5 text-[10px] font-semibold tracking-wide;
    @apply px-1.5 py-0.5 rounded bg-red-600 text-white pointer-events-none;
  }

  .delete {
    @apply absolute top-1 right-1 z-30 p-0.5 rounded bg-zinc-800/80 text-zinc-300;
    @apply opacity-0 transition-opacity hover:bg-red-600 hover:text-white;
  }

  .board-item:hover .delete {
    @apply opacity-100;
  }

  .download {
    @apply absolute bottom-1 left-1 p-0.5 rounded bg-zinc-800/80 text-zinc-300 z-30;
    @apply opacity-0 transition-opacity hover:bg-indigo-600 hover:text-white;
  }

  .drag-grip {
    @apply absolute top-1 left-1 z-20 rounded bg-zinc-800/80 text-zinc-300;
    @apply px-1.5 py-0.5 text-xs leading-none cursor-move touch-none select-none;
    @apply opacity-0 transition-opacity;
  }

  .board-item:hover .drag-grip {
    @apply opacity-100;
  }

  .board-item:hover .download {
    @apply opacity-100;
  }

  .resize-handle {
    @apply absolute bottom-0 right-0 w-4 h-4 z-20 cursor-nwse-resize touch-none;
    @apply opacity-0 transition-opacity;
    background: linear-gradient(
      135deg,
      transparent 0%,
      transparent 50%,
      theme("colors.indigo.400") 50%,
      theme("colors.indigo.400") 100%
    );
    border-bottom-right-radius: theme("borderRadius.lg");
  }

  .board-item:hover .resize-handle,
  .resize-handle.resizing {
    @apply opacity-90;
  }

  /* Touch devices have no hover — keep board-item controls visible + finger-sized. */
  @media (hover: none), (pointer: coarse) {
    .delete,
    .download,
    .drag-grip {
      @apply opacity-100 p-1.5;
    }
    .resize-handle {
      @apply opacity-90 w-11 h-11;
    }
    .drag-grip {
      @apply min-w-[44px] min-h-[44px] grid place-items-center text-base;
    }
  }
</style>
