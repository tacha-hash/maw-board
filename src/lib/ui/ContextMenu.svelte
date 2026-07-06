<!--
  Right-click "New Node" menu for the canvas (PLAN.md round-2 spec:
  "คลิกขวาบน canvas = New Node"). Positioned at the click point in screen
  space by the parent (Session.svelte computes both screen + world coords
  from the same MouseEvent via normalizePosition).
-->
<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import {
    TerminalIcon,
    FileTextIcon,
    ImageIcon,
    UsersIcon,
  } from "svelte-feather-icons";

  export let x: number;
  export let y: number;

  const dispatch = createEventDispatcher<{
    terminal: void;
    note: void;
    imageGen: void;
    meeting: void;
    close: void;
  }>();

  function pick(kind: "terminal" | "note" | "imageGen" | "meeting") {
    dispatch(kind);
    dispatch("close");
  }
</script>

<svelte:window on:keydown={(event) => event.key === "Escape" && dispatch("close")} />

<!-- Full-screen backdrop closes the menu on outside click; also swallows a
     second right-click so it doesn't re-trigger the browser's native menu. -->
<button
  class="backdrop"
  aria-label="Close menu"
  on:click={() => dispatch("close")}
  on:contextmenu|preventDefault={() => dispatch("close")}
/>

<div class="menu panel" style:left="{x}px" style:top="{y}px">
  <button class="item" on:click={() => pick("terminal")}>
    <TerminalIcon size="15" />
    New Terminal
  </button>
  <button class="item" on:click={() => pick("note")}>
    <FileTextIcon size="15" />
    New Note
  </button>
  <button class="item" on:click={() => pick("imageGen")}>
    <ImageIcon size="15" />
    New Image-gen Node
  </button>
  <button class="item disabled" disabled title="Coming soon">
    <UsersIcon size="15" />
    New Meeting
    <span class="soon">soon</span>
  </button>
</div>

<style lang="postcss">
  .backdrop {
    @apply fixed inset-0 z-40 cursor-default bg-transparent;
  }
  .menu {
    @apply fixed z-50 flex flex-col gap-0.5 p-1.5 min-w-[190px];
  }
  .item {
    @apply flex items-center gap-2 px-2.5 py-1.5 rounded-md text-sm text-zinc-200 text-left;
    @apply hover:bg-white/10;
  }
  .item.disabled {
    @apply text-zinc-500 cursor-not-allowed hover:bg-transparent;
  }
  .soon {
    @apply ml-auto text-[10px] text-zinc-500;
  }
</style>
