<!--
  VR4 — single-node Detail popover (docs/vision-round4-node-detail-design.md).
  Opens on right-click of a node that isn't part of a multi-selection. Shows a
  universal geometry editor (X/Y/W/H) + per-kind details, plus Duplicate/Delete.
  Same popover family as ContextMenu.svelte (full-screen backdrop closes it),
  but clamped to stay on-screen (Le review). Read-only when !canEdit.
-->
<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { XIcon, DownloadIcon, CopyIcon, Trash2Icon, ExternalLinkIcon } from "svelte-feather-icons";
  import type { BoardItem } from "../protocol";
  import { parseJobPayload } from "../jobPayload";
  import { parseOrderPayload } from "../orderPayload";

  export let item: BoardItem;
  export let canEdit: boolean;
  export let x: number; // screen anchor (click point)
  export let y: number;
  /** Whether a done job has a rendered result tile (<id>-out) to jump to. */
  export let hasResult = false;
  /** Per-agent status for an order node, joined from its order-status sibling. */
  export let orderStatus: { agent: string; state: string }[] = [];

  const dispatch = createEventDispatcher<{
    move: { x: number; y: number };
    resize: { w: number; h: number };
    noteEdit: string;
    duplicate: void;
    delete: void;
    openResult: void;
    close: void;
  }>();

  const MIN_SIZE = 40;
  const shortId = item.id.length > 10 ? `${item.id.slice(0, 6)}…` : item.id;

  $: job = item.kind === "job" ? parseJobPayload(item.dataUrl) : null;
  $: order = item.kind === "order" ? parseOrderPayload(item.dataUrl) : null;

  const KIND_META: Record<string, { icon: string; label: string }> = {
    image: { icon: "🖼", label: "Image" },
    video: { icon: "🎬", label: "Video" },
    note: { icon: "📝", label: "Note" },
    order: { icon: "📋", label: "Work Order" },
    stream: { icon: "🖥", label: "Live Stream" },
  };
  $: meta =
    item.kind === "job"
      ? { icon: job?.media_type === "video" ? "🎬" : "🎨", label: job?.media_type === "video" ? "Video Gen" : "Image Gen" }
      : KIND_META[item.kind] ?? { icon: "▦", label: item.kind };

  // Only content nodes can be duplicated/deleted from here; a live stream tile
  // is a peer-owned placeholder, so we leave it read-only.
  $: canMutate = canEdit && item.kind !== "stream";

  function commitNum(field: "x" | "y" | "w" | "h", value: number) {
    if (!canEdit || !Number.isFinite(value)) return;
    if (field === "x") dispatch("move", { x: Math.round(value), y: item.y });
    else if (field === "y") dispatch("move", { x: item.x, y: Math.round(value) });
    else if (field === "w") dispatch("resize", { w: Math.max(MIN_SIZE, Math.round(value)), h: item.h });
    else dispatch("resize", { w: item.w, h: Math.max(MIN_SIZE, Math.round(value)) });
  }

  // Keep the popover fully on-screen: after it renders, clamp its top-left
  // against the measured size so a right-click near an edge doesn't push it out.
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
    return {
      update: apply,
      destroy: () => window.removeEventListener("resize", onResize),
    };
  }
</script>

<svelte:window on:keydown={(event) => event.key === "Escape" && dispatch("close")} />

<button
  class="backdrop"
  aria-label="Close detail"
  on:click={() => dispatch("close")}
  on:contextmenu|preventDefault={() => dispatch("close")}
></button>

<div class="detail panel" use:clamp={{ x, y }}>
  <div class="head">
    <span class="title"><span class="icon">{meta.icon}</span> {meta.label}</span>
    <span class="id" title={item.id}>#{shortId}</span>
    <button class="x" aria-label="Close" on:click={() => dispatch("close")}>
      <XIcon size="14" />
    </button>
  </div>

  <!-- ── Per-kind details ── -->
  {#if item.kind === "image" || item.kind === "video"}
    <div class="section preview">
      {#if item.kind === "video"}
        <!-- svelte-ignore a11y-media-has-caption -->
        <video src={item.dataUrl} class="media" controls muted playsinline></video>
      {:else}
        <img src={item.dataUrl} class="media" alt="node preview" />
      {/if}
      <a class="btn" href={item.dataUrl} download={`${item.kind}-${item.id.slice(0, 8)}`}>
        <DownloadIcon size="13" /> Download
      </a>
    </div>
  {:else if item.kind === "note"}
    <div class="section">
      <span class="label">Text</span>
      <textarea
        class="note-edit"
        placeholder="Type a note…"
        value={item.dataUrl}
        readonly={!canEdit}
        on:input={(e) => canEdit && dispatch("noteEdit", e.currentTarget.value)}
      ></textarea>
    </div>
  {:else if item.kind === "job" && job}
    <div class="section">
      <div class="kv"><span class="label">State</span><span class="badge state-{job.state}">{job.state}</span></div>
      <div class="kv"><span class="label">Model</span><span class="val">{job.model}</span></div>
      <div class="kv"><span class="label">Provider</span><span class="val">{job.provider}</span></div>
      {#if job.prompt.trim()}
        <div class="prompt-snip" title={job.prompt}>{job.prompt}</div>
      {:else}
        <div class="muted">No prompt yet</div>
      {/if}
      <p class="hint">Edit generation settings on the node itself.</p>
      {#if job.state === "done" && hasResult}
        <button class="btn" on:click={() => dispatch("openResult")}>
          <ExternalLinkIcon size="13" /> Open result
        </button>
      {/if}
    </div>
  {:else if item.kind === "order" && order}
    <div class="section">
      <div class="kv"><span class="label">Title</span><span class="val">{order.title || "(untitled)"}</span></div>
      <div class="kv"><span class="label">Dispatched</span><span class="val">{order.dispatch_seq > 0 ? `×${order.dispatch_seq}` : "not yet"}</span></div>
      {#if orderStatus.length}
        <span class="label">Assignees</span>
        <div class="assignees">
          {#each orderStatus as a}
            <div class="kv"><span class="val">{a.agent}</span><span class="badge state-{a.state}">{a.state}</span></div>
          {/each}
        </div>
      {/if}
      <p class="hint">Edit title / prompt on the node itself.</p>
    </div>
  {:else if item.kind === "stream"}
    <div class="section"><p class="muted">Live screen share (read-only).</p></div>
  {/if}

  <!-- ── Geometry (universal, editable) ── -->
  <div class="section">
    <span class="label">Position &amp; size</span>
    <div class="geo">
      <label>X<input type="number" value={item.x} disabled={!canEdit}
        on:change={(e) => commitNum("x", e.currentTarget.valueAsNumber)} /></label>
      <label>Y<input type="number" value={item.y} disabled={!canEdit}
        on:change={(e) => commitNum("y", e.currentTarget.valueAsNumber)} /></label>
      <label>W<input type="number" value={item.w} min={MIN_SIZE} disabled={!canEdit}
        on:change={(e) => commitNum("w", e.currentTarget.valueAsNumber)} /></label>
      <label>H<input type="number" value={item.h} min={MIN_SIZE} disabled={!canEdit}
        on:change={(e) => commitNum("h", e.currentTarget.valueAsNumber)} /></label>
    </div>
  </div>

  <!-- ── Actions ── -->
  {#if canMutate}
    <div class="actions">
      <button class="btn" on:click={() => dispatch("duplicate")}>
        <CopyIcon size="13" /> Duplicate
      </button>
      <button class="btn danger" on:click={() => dispatch("delete")}>
        <Trash2Icon size="13" /> Delete
      </button>
    </div>
  {/if}
</div>

<style lang="postcss">
  .backdrop {
    @apply fixed inset-0 z-40 cursor-default bg-transparent;
  }
  .detail {
    @apply fixed z-50 flex flex-col gap-2 p-2.5 w-[248px] max-h-[80vh] overflow-y-auto;
  }
  .head {
    @apply flex items-center gap-2 pb-1.5 border-b border-white/10;
  }
  .title {
    @apply flex items-center gap-1.5 text-sm font-semibold text-zinc-100;
  }
  .icon {
    @apply text-base leading-none;
  }
  .id {
    @apply ml-auto text-[10px] font-mono text-zinc-500;
  }
  .x {
    @apply p-0.5 rounded text-zinc-400 hover:bg-white/10 hover:text-zinc-100;
  }
  .section {
    @apply flex flex-col gap-1.5;
  }
  .label {
    @apply text-[10px] uppercase tracking-wide text-zinc-500;
  }
  .kv {
    @apply flex items-center justify-between gap-2 text-xs;
  }
  .val {
    @apply text-zinc-200 truncate;
  }
  .muted {
    @apply text-xs text-zinc-500 italic;
  }
  .hint {
    @apply text-[10px] text-zinc-500;
  }
  .badge {
    @apply px-1.5 py-0.5 rounded text-[10px] font-medium bg-white/10 text-zinc-300;
  }
  .state-done {
    @apply bg-emerald-500/20 text-emerald-300;
  }
  .state-error {
    @apply bg-red-500/20 text-red-300;
  }
  .state-running,
  .state-working,
  .state-pending {
    @apply bg-amber-500/20 text-amber-300;
  }
  .preview {
    @apply items-stretch;
  }
  .media {
    @apply w-full max-h-40 object-contain rounded bg-black/30;
  }
  .prompt-snip {
    @apply text-xs text-zinc-300 line-clamp-3 bg-black/20 rounded p-1.5;
  }
  .note-edit {
    @apply w-full h-24 resize-none rounded bg-black/20 p-1.5 text-xs text-zinc-100 outline-none focus:ring-1 focus:ring-amber-400/50;
  }
  .assignees {
    @apply flex flex-col gap-1;
  }
  .geo {
    @apply grid grid-cols-2 gap-1.5;
  }
  .geo label {
    @apply flex items-center gap-1 text-[11px] text-zinc-400;
  }
  .geo input {
    @apply w-full rounded bg-black/25 px-1.5 py-1 text-xs text-zinc-100 outline-none focus:ring-1 focus:ring-amber-400/50 disabled:opacity-50;
  }
  .actions {
    @apply flex gap-1.5 pt-1;
  }
  .btn {
    @apply flex items-center justify-center gap-1.5 px-2 py-1.5 rounded-md text-xs text-zinc-200 bg-white/5 hover:bg-white/10 flex-1;
  }
  .btn.danger {
    @apply text-red-300 hover:bg-red-500/15;
  }
</style>
