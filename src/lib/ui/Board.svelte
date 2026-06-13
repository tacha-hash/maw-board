<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { fade } from "svelte/transition";
  import { XIcon, DownloadIcon } from "svelte-feather-icons";

  import { slide } from "../action/slide";
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

  const dispatch = createEventDispatcher<{
    move: { id: string; x: number; y: number };
    resize: { id: string; w: number; h: number };
    delete: string;
    edit: { id: string; text: string };
  }>();

  // Resize state — drag the bottom-right handle to stretch a tile (like a window).
  const MIN_W = 120;
  const MIN_H = 80;
  let resizeId: string | null = null;
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
    if (resizeId === null) return;
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
    if (resizeId !== null) {
      const { w, h } = currentResize(event);
      dispatch("resize", { id: resizeId, w, h });
    }
    resizeId = null;
    latestResizeEvent = null;
    window.removeEventListener("pointermove", onResize);
    window.removeEventListener("pointerup", endResize);
    window.removeEventListener("pointercancel", endResize);
  }

  // Drag state. While dragging, the dragged tile renders at `dragPos` and sends
  // BoardMove on a requestAnimationFrame cadence (contract v2: client throttle).
  const LONG_PRESS_MS = 400;

  let dragId: string | null = null;
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
    // Desktop (mouse): start drag immediately.
    if (event.pointerType === "mouse") {
      startDrag(event, item);
      return;
    }
    // Touch: wait for long-press before allowing drag.
    pressItem = item;
    pressEvent = event;
    longPressActive = false;
    pressTimer = setTimeout(() => {
      if (pressItem && pressEvent) {
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
      onMove(event);
      return;
    }
    // If finger moves too far before long-press fires, cancel.
    if (pressEvent) {
      const dx = event.clientX - pressEvent.clientX;
      const dy = event.clientY - pressEvent.clientY;
      if (dx * dx + dy * dy > 100) cancelPress();
    }
  }

  function cancelPress() {
    if (pressTimer) clearTimeout(pressTimer);
    pressTimer = null;
    pressItem = null;
    pressEvent = null;
    if (longPressActive) {
      endDrag();
      longPressActive = false;
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
    dragOffset = [wx - item.x, wy - item.y];
    dragPos = [item.x, item.y];
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", endDrag);
  }

  function onMove(event: PointerEvent) {
    if (dragId === null) return;
    const [wx, wy] = normalizePosition(event);
    dragPos = [Math.round(wx - dragOffset[0]), Math.round(wy - dragOffset[1])];
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

  function endDrag() {
    if (dragId !== null) {
      dispatch("move", { id: dragId, x: dragPos[0], y: dragPos[1] });
    }
    dragId = null;
    longPressActive = false;
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", endDrag);
  }
</script>

{#each items.filter((it) => it.kind !== "doc" && it.kind !== "lock" && it.kind !== "label") as item (item.id)}
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
      style:width="{item.w}px"
      on:pointerdown={(event) => onPointerDown(event, item)}
    >
      {#if item.kind === "note"}
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

      {#if item.kind === "video"}
        <a
          class="download"
          href={item.dataUrl}
          download="video"
          title="Download video"
          on:pointerdown={(event) => event.stopPropagation()}
        >
          <DownloadIcon size="14" />
        </a>
      {/if}

      {#if hasWriteAccess !== false}
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

<style lang="postcss">
  .board-item {
    @apply relative rounded-lg overflow-hidden bg-zinc-900 shadow-lg cursor-move select-none;
    @apply ring-1 ring-zinc-700 transition-transform duration-150;
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
    @apply absolute top-1 right-1 p-0.5 rounded bg-zinc-800/80 text-zinc-300;
    @apply opacity-0 transition-opacity hover:bg-red-600 hover:text-white;
  }

  .board-item:hover .delete {
    @apply opacity-100;
  }

  .download {
    @apply absolute top-1 left-1 p-0.5 rounded bg-zinc-800/80 text-zinc-300 z-10;
    @apply opacity-0 transition-opacity hover:bg-indigo-600 hover:text-white;
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
</style>
