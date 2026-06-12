<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { fade } from "svelte/transition";
  import { XIcon } from "svelte-feather-icons";

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
    delete: string;
  }>();

  // Drag state. While dragging, the dragged tile renders at `dragPos` and sends
  // BoardMove on a requestAnimationFrame cadence (contract v2: client throttle).
  let dragId: string | null = null;
  let dragOffset = [0, 0];
  let dragPos = [0, 0];
  let rafPending = false;

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
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", endDrag);
  }
</script>

{#each items as item (item.id)}
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
      style:width="{item.w}px"
      on:pointerdown={(event) => startDrag(event, item)}
    >
      <img
        src={streamSrcs[item.id] ?? item.dataUrl}
        alt={item.kind === "stream" ? "screen share" : "shared image"}
        draggable="false"
      />

      {#if item.kind === "stream"}
        <div class="live-tag">● LIVE</div>
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
      {/if}
    </div>
  </div>
{/each}

<style lang="postcss">
  .board-item {
    @apply relative rounded-lg overflow-hidden bg-zinc-900 shadow-lg cursor-move select-none;
    @apply ring-1 ring-zinc-700;
  }

  .board-item.is-stream {
    @apply ring-2 ring-red-500/70;
  }

  .board-item img {
    @apply block w-full h-auto pointer-events-none;
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
</style>
