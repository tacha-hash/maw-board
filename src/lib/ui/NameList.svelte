<script lang="ts">
  import { flip } from "svelte/animate";

  import type { WsUser } from "$lib/protocol";
  import { nameToHue } from "./LiveCursor.svelte";

  export let users: [number, WsUser][];
  export let vertical: boolean = false;
  $: sortedUsers = [...users].sort(
    (a, b) => Number(b[1].canWrite) - Number(a[1].canWrite),
  );
</script>

<ul class="flex gap-1" class:flex-col={vertical} class:flex-wrap={!vertical}>
  {#each sortedUsers as [id, user] (id)}
    <li
      class="flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium bg-zinc-800/80 border border-zinc-700/50"
      class:opacity-60={!user.canWrite}
      animate:flip={{ duration: 200 }}
    >
      <div
        style:background="hsl({nameToHue(user.name)}, 70%, 55%)"
        class="w-2 h-2 rounded-full flex-shrink-0"
      />
      <span class="text-zinc-300 truncate max-w-[80px]">{user.name}</span>
    </li>
  {/each}
</ul>
