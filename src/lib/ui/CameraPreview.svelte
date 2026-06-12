<!--
  Floating, draggable + resizable local camera preview window.
  Appears when the user turns their camera on. Works with both mouse and
  touch (long-press-free direct drag on the header; pinch-free corner resize).
-->
<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { XIcon, VideoIcon } from "svelte-feather-icons";

  export let stream: MediaStream;
  export let label: string = "Camera";
  export let mirror: boolean = true; // mirror local self-view, not remote peers
  export let closable: boolean = true;
  export let index: number = 0; // stagger multiple windows so they don't overlap

  const dispatch = createEventDispatcher<{ close: void }>();

  // Window geometry (px). Default: stack down the top-right corner.
  let x =
    (typeof window !== "undefined" ? window.innerWidth - 320 : 40) -
    index * 24;
  let y = 84 + index * 232;
  let w = 288;
  let h = 216;

  const MIN_W = 160;
  const MIN_H = 120;

  // Bind the MediaStream to the <video> element (srcObject is not an attribute).
  function bindStream(node: HTMLVideoElement, s: MediaStream) {
    node.srcObject = s;
    return {
      update(s2: MediaStream) {
        node.srcObject = s2;
      },
    };
  }

  function startDrag(event: PointerEvent) {
    if (event.button !== undefined && event.button !== 0) return;
    event.preventDefault();
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    const ox = event.clientX - x;
    const oy = event.clientY - y;
    function onMove(e: PointerEvent) {
      x = Math.max(0, Math.min(window.innerWidth - 40, e.clientX - ox));
      y = Math.max(0, Math.min(window.innerHeight - 40, e.clientY - oy));
    }
    function onUp(e: PointerEvent) {
      target.releasePointerCapture(event.pointerId);
      target.removeEventListener("pointermove", onMove);
      target.removeEventListener("pointerup", onUp);
    }
    target.addEventListener("pointermove", onMove);
    target.addEventListener("pointerup", onUp);
  }

  function startResize(event: PointerEvent) {
    event.preventDefault();
    event.stopPropagation();
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    const ow = w - event.clientX;
    const oh = h - event.clientY;
    function onMove(e: PointerEvent) {
      w = Math.max(MIN_W, ow + e.clientX);
      h = Math.max(MIN_H, oh + e.clientY);
    }
    function onUp() {
      target.releasePointerCapture(event.pointerId);
      target.removeEventListener("pointermove", onMove);
      target.removeEventListener("pointerup", onUp);
    }
    target.addEventListener("pointermove", onMove);
    target.addEventListener("pointerup", onUp);
  }
</script>

<div
  class="cam-window panel"
  style:left={`${x}px`}
  style:top={`${y}px`}
  style:width={`${w}px`}
  style:height={`${h}px`}
>
  <div class="cam-header" on:pointerdown={startDrag}>
    <div class="flex items-center gap-1.5 text-xs text-zinc-300 font-medium">
      <VideoIcon size="14" />
      <span>{label}</span>
    </div>
    {#if closable}
      <button class="cam-close" title="Turn camera off" on:click={() => dispatch("close")}>
        <XIcon size="14" />
      </button>
    {/if}
  </div>

  <!-- svelte-ignore a11y-media-has-caption -->
  <video
    class="cam-video"
    class:mirror
    autoplay
    playsinline
    muted
    use:bindStream={stream}
  />

  <!-- Resize handle (bottom-right) -->
  <div class="cam-resize" on:pointerdown={startResize} title="Resize" />
</div>

<style lang="postcss">
  .cam-window {
    @apply fixed z-50 flex flex-col overflow-hidden p-0;
  }

  .cam-header {
    @apply flex items-center justify-between px-2.5 py-1.5 cursor-move select-none;
    @apply bg-zinc-800/70 border-b border-zinc-700/60;
    touch-action: none;
  }

  .cam-close {
    @apply rounded-md p-0.5 text-zinc-400 hover:text-white hover:bg-zinc-700/60 transition-colors;
  }

  .cam-video {
    @apply flex-1 w-full h-full object-cover bg-black;
  }

  .cam-video.mirror {
    transform: scaleX(-1); /* mirror local self-view like a selfie */
  }

  .cam-resize {
    @apply absolute bottom-0 right-0 w-4 h-4 cursor-nwse-resize;
    touch-action: none;
    background: linear-gradient(135deg, transparent 50%, rgb(113 113 122 / 0.8) 50%);
  }
</style>
