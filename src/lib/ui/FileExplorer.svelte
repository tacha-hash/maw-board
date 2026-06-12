<!--
  IDE-style file explorer sidebar. Browses the server's shared workspace
  (read-only) via /api/files, expanding folders on click. Svelte 3 compatible:
  the tree is flattened into visible rows rather than recursive components.
-->
<script lang="ts">
  import { createEventDispatcher, onMount } from "svelte";
  import {
    XIcon,
    FolderIcon,
    FileIcon,
    ChevronRightIcon,
    ChevronDownIcon,
  } from "svelte-feather-icons";

  const dispatch = createEventDispatcher<{ close: void }>();

  type Entry = { name: string; dir: boolean; size: number };
  type Row = { entry: Entry; path: string; depth: number };

  let children: Record<string, Entry[]> = {};
  let expanded: Record<string, boolean> = {};
  let error = "";

  async function fetchDir(path: string) {
    if (children[path]) return;
    try {
      const r = await fetch(`/api/files?path=${encodeURIComponent(path)}`);
      if (!r.ok) throw new Error(await r.text());
      const data = await r.json();
      children = { ...children, [path]: data.items as Entry[] };
      error = "";
    } catch (e) {
      error = e instanceof Error ? e.message : "failed to load";
    }
  }

  function toggle(path: string) {
    if (expanded[path]) {
      expanded = { ...expanded, [path]: false };
    } else {
      expanded = { ...expanded, [path]: true };
      fetchDir(path);
    }
  }

  function humanSize(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
    return `${(n / 1024 / 1024).toFixed(1)} MB`;
  }

  // Flatten the expanded tree into an ordered list of visible rows.
  function flatten(
    cache: Record<string, Entry[]>,
    open: Record<string, boolean>,
  ): Row[] {
    const rows: Row[] = [];
    const walk = (parent: string, depth: number) => {
      for (const entry of cache[parent] ?? []) {
        const path = parent ? `${parent}/${entry.name}` : entry.name;
        rows.push({ entry, path, depth });
        if (entry.dir && open[path]) walk(path, depth + 1);
      }
    };
    walk("", 0);
    return rows;
  }

  $: rows = flatten(children, expanded);

  onMount(() => fetchDir(""));
</script>

<div class="explorer panel">
  <div class="head">
    <div class="flex items-center gap-1.5 text-zinc-200 font-medium text-sm">
      <FolderIcon size="15" />
      <span>Files</span>
      <span class="text-[10px] text-zinc-500">maw-workspace</span>
    </div>
    <button class="close" title="Close" on:click={() => dispatch("close")}>
      <XIcon size="15" />
    </button>
  </div>

  <div class="tree">
    {#if error}
      <p class="err">{error}</p>
    {/if}
    {#if !children[""]}
      <p class="muted">Loading…</p>
    {/if}
    {#each rows as row (row.path)}
      <div
        class="row"
        style:padding-left="{row.depth * 14 + 8}px"
        role="button"
        tabindex="0"
        on:click={() => row.entry.dir && toggle(row.path)}
        on:keydown={(e) => e.key === "Enter" && row.entry.dir && toggle(row.path)}
      >
        {#if row.entry.dir}
          <span class="chev">
            {#if expanded[row.path]}
              <ChevronDownIcon size="13" />
            {:else}
              <ChevronRightIcon size="13" />
            {/if}
          </span>
          <FolderIcon size="14" class="text-indigo-300" />
        {:else}
          <span class="chev" />
          <FileIcon size="14" class="text-zinc-400" />
        {/if}
        <span class="name" class:dir={row.entry.dir}>{row.entry.name}</span>
        {#if !row.entry.dir}
          <span class="size">{humanSize(row.entry.size)}</span>
        {/if}
      </div>
    {/each}
  </div>
</div>

<style lang="postcss">
  .explorer {
    @apply fixed left-4 top-20 bottom-4 w-72 z-40 flex flex-col p-0 overflow-hidden;
  }
  .head {
    @apply flex items-center justify-between px-3 py-2 border-b border-zinc-700/60;
  }
  .close {
    @apply rounded-md p-0.5 text-zinc-400 hover:text-white hover:bg-zinc-700/60;
  }
  .tree {
    @apply flex-1 overflow-y-auto py-1 text-sm;
  }
  .row {
    @apply flex items-center gap-1 pr-2 py-1 cursor-pointer hover:bg-white/5;
  }
  .chev {
    @apply w-3.5 flex-none text-zinc-500;
  }
  .name {
    @apply truncate text-zinc-300;
  }
  .name.dir {
    @apply text-zinc-100;
  }
  .size {
    @apply ml-auto text-[10px] text-zinc-600 flex-none pl-2;
  }
  .muted {
    @apply px-3 py-1 text-xs text-zinc-500;
  }
  .err {
    @apply px-3 py-1 text-xs text-red-400;
  }
</style>
