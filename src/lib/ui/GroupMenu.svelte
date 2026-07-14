<!--
  VR4 — Group menu (docs/vision-round4-node-detail-design.md). Opens on
  right-click of a node that IS part of a marquee multi-selection; acts on the
  whole selection. Align/distribute + duplicate + delete (light two-step
  confirm, Le review). "Create group" is the persist-group affordance (phase b,
  needs bridge sync) — shown disabled/"soon" like New Meeting in ContextMenu.
-->
<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { CopyIcon, Trash2Icon, LayersIcon } from "svelte-feather-icons";
  import type { AlignEdge, DistributeAxis } from "../align";

  export let x: number; // screen anchor
  export let y: number;
  export let count: number;
  export let canEdit: boolean;

  const dispatch = createEventDispatcher<{
    align: AlignEdge;
    distribute: DistributeAxis;
    duplicate: void;
    delete: void;
    close: void;
  }>();

  let confirmDelete = false;

  const ALIGN: { edge: AlignEdge; glyph: string; title: string }[] = [
    { edge: "left", glyph: "⇤", title: "Align left" },
    { edge: "center-h", glyph: "⇔", title: "Align centre (horizontal)" },
    { edge: "right", glyph: "⇥", title: "Align right" },
    { edge: "top", glyph: "⤒", title: "Align top" },
    { edge: "middle-v", glyph: "⇕", title: "Align middle (vertical)" },
    { edge: "bottom", glyph: "⤓", title: "Align bottom" },
  ];

  function onDelete() {
    if (!confirmDelete) {
      confirmDelete = true;
      return;
    }
    dispatch("delete");
    dispatch("close");
  }

  function clamp(node: HTMLElement, pos: { x: number; y: number }) {
    const apply = (p: { x: number; y: number }) => {
      const r = node.getBoundingClientRect();
      const pad = 8;
      const left = Math.max(pad, Math.min(p.x, window.innerWidth - r.width - pad));
      const top = Math.max(pad, Math.min(p.y, window.innerHeight - r.height - pad));
      node.style.left = `${left}px`;
      node.style.top = `${top}px`;
    };
    apply(pos);
    const onResize = () => apply(pos);
    window.addEventListener("resize", onResize);
    return { update: apply, destroy: () => window.removeEventListener("resize", onResize) };
  }
</script>

<svelte:window on:keydown={(event) => event.key === "Escape" && dispatch("close")} />

<button
  class="backdrop"
  aria-label="Close group menu"
  on:click={() => dispatch("close")}
  on:contextmenu|preventDefault={() => dispatch("close")}
></button>

<div class="group panel" use:clamp={{ x, y }}>
  <div class="head">{count} nodes selected</div>

  {#if canEdit}
    <span class="label">Align</span>
    <div class="align-grid">
      {#each ALIGN as a}
        <button class="glyph-btn" title={a.title} on:click={() => dispatch("align", a.edge)}>{a.glyph}</button>
      {/each}
    </div>

    <span class="label">Distribute</span>
    <div class="dist-row">
      <button class="item" disabled={count < 3} on:click={() => dispatch("distribute", "h")}>Horizontal</button>
      <button class="item" disabled={count < 3} on:click={() => dispatch("distribute", "v")}>Vertical</button>
    </div>

    <div class="sep"></div>

    <button class="item" on:click={() => { dispatch("duplicate"); dispatch("close"); }}>
      <CopyIcon size="14" /> Duplicate group
    </button>
    <button class="item danger" class:confirm={confirmDelete} on:click={onDelete}>
      <Trash2Icon size="14" />
      {confirmDelete ? `Click again to delete ${count}` : `Delete group (${count})`}
    </button>

    <div class="sep"></div>

    <button class="item disabled" disabled title="Persist a named group — coming soon (needs bridge sync)">
      <LayersIcon size="14" /> Create group
      <span class="soon">soon</span>
    </button>
  {:else}
    <p class="muted">View-only — no group actions.</p>
  {/if}
</div>

<style lang="postcss">
  .backdrop {
    @apply fixed inset-0 z-40 cursor-default bg-transparent;
  }
  .group {
    @apply fixed z-50 flex flex-col gap-1 p-1.5 min-w-[200px];
  }
  .head {
    @apply px-1.5 py-1 text-xs font-semibold text-zinc-100;
  }
  .label {
    @apply px-1.5 pt-1 text-[10px] uppercase tracking-wide text-zinc-500;
  }
  .align-grid {
    @apply grid grid-cols-6 gap-1 px-1;
  }
  .glyph-btn {
    @apply flex items-center justify-center py-1 rounded-md text-base text-zinc-200 bg-white/5 hover:bg-white/10 leading-none;
  }
  .dist-row {
    @apply flex gap-1 px-1;
  }
  .dist-row .item {
    @apply flex-1 justify-center;
  }
  .item {
    @apply flex items-center gap-2 px-2.5 py-1.5 rounded-md text-sm text-zinc-200 text-left;
    @apply hover:bg-white/10 disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-transparent;
  }
  .item.danger {
    @apply text-red-300 hover:bg-red-500/15;
  }
  .item.danger.confirm {
    @apply bg-red-500/20 text-red-200;
  }
  .item.disabled {
    @apply text-zinc-500 cursor-not-allowed hover:bg-transparent;
  }
  .soon {
    @apply ml-auto text-[10px] text-zinc-500;
  }
  .sep {
    @apply my-0.5 border-t border-white/10;
  }
  .muted {
    @apply px-2 py-1.5 text-xs text-zinc-500 italic;
  }
</style>
