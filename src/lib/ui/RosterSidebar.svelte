<!--
  Left sidebar: agent roster (PLAN.md round-2 spec point 2 — "Sidebar ซ้าย =
  จัดการ agent"). Read-only list of agents comes from the bridge-owned
  `kind:"roster"` singleton board item — this component never writes to it.
  "Add to board" / "New agent" only ever post a `kind:"agent-request"` item;
  actually spawning a mirror/terminal is entirely the bridge's job.
-->
<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { XIcon, UsersIcon, PlusIcon } from "svelte-feather-icons";

  export let agents: { name: string; host?: string; status?: string }[] = [];
  export let canEdit: boolean;
  // Names currently mirrored on the board (matched against terminal labels
  // by Session.svelte) — toggles each row between View/Interact and Remove.
  export let onBoard: Set<string> = new Set();

  const dispatch = createEventDispatcher<{
    close: void;
    addAgent: { name: string; mode: "view" | "interact" };
    removeAgent: string;
    createAgent: { name: string; host: string; repo: string };
  }>();

  let showForm = false;
  let formName = "";
  let formHost = "";
  let formRepo = "";

  function submitForm() {
    if (!formName.trim()) return;
    dispatch("createAgent", {
      name: formName.trim(),
      host: formHost.trim(),
      repo: formRepo.trim(),
    });
    formName = "";
    formHost = "";
    formRepo = "";
    showForm = false;
  }
</script>

<div class="roster panel">
  <div class="head">
    <div class="flex items-center gap-1.5 text-zinc-200 font-medium text-sm">
      <UsersIcon size="15" />
      <span>Agents</span>
    </div>
    <button class="close" title="Close" on:click={() => dispatch("close")}>
      <XIcon size="15" />
    </button>
  </div>

  <div class="list">
    {#if agents.length === 0}
      <p class="muted">Waiting for roster from bridge…</p>
    {/if}
    {#each agents as agent (agent.name)}
      {@const mirrored = onBoard.has(agent.name)}
      <div class="row">
        <span class="dot" class:online={mirrored} />
        <span class="name" title={agent.host ?? ""}>{agent.name}</span>
        {#if mirrored}
          <button
            class="add-btn remove-btn"
            disabled={!canEdit}
            title="Remove from board"
            on:click={() => dispatch("removeAgent", agent.name)}
          >
            Remove
          </button>
        {:else}
          <!-- Louis's call (2026-07-06 ค่ำ, reversed an earlier direction of
               mine): Add's primary action IS a writable terminal by default
               — typing reaches the real agent, "like sitting in front of
               the machine". View (read-only) is the secondary, icon-only
               button for when you just want to watch. -->
          <div class="actions">
            <button
              class="icon-btn"
              disabled={!canEdit}
              title="View (read-only)"
              on:click={() => dispatch("addAgent", { name: agent.name, mode: "view" })}
            >
              👁
            </button>
            <button
              class="add-btn interact-btn"
              disabled={!canEdit}
              title="Add — writable, typing reaches the real agent, like sitting in front of the machine"
              on:click={() => dispatch("addAgent", { name: agent.name, mode: "interact" })}
            >
              + Add
            </button>
          </div>
        {/if}
      </div>
    {/each}
  </div>

  {#if showForm}
    <div class="form">
      <input class="field" placeholder="Agent name" bind:value={formName} />
      <input class="field" placeholder="Host (e.g. pc, mac)" bind:value={formHost} />
      <input class="field" placeholder="Repo (optional)" bind:value={formRepo} />
      <div class="form-actions">
        <button class="btn" on:click={() => (showForm = false)}>Cancel</button>
        <button
          class="btn btn-primary"
          disabled={!formName.trim()}
          on:click={submitForm}
        >
          Request
        </button>
      </div>
    </div>
  {:else}
    <button class="new-agent" disabled={!canEdit} on:click={() => (showForm = true)}>
      <PlusIcon size="14" />
      New agent
    </button>
  {/if}
</div>

<style lang="postcss">
  /* top-36 (not top-20, like FileExplorer) — the "online users" NameList
     (Session.svelte, `fixed left-3 top-24`) occupies the same top-left
     corner and Roster defaults open (unlike FileExplorer), so it collides
     with NameList's rows unless pushed down past its typical height. */
  .roster {
    @apply fixed left-2 right-2 sm:right-auto sm:left-4 top-36 bottom-4 w-auto sm:w-72 z-30 flex flex-col p-0 overflow-hidden;
  }
  .head {
    @apply flex items-center justify-between px-3 py-2 border-b border-zinc-700/60;
  }
  .close {
    @apply rounded-md p-0.5 text-zinc-400 hover:text-white hover:bg-zinc-700/60;
  }
  .list {
    @apply flex-1 overflow-y-auto py-1 text-sm;
  }
  .row {
    @apply flex items-center gap-1.5 px-3 py-1.5 hover:bg-white/5;
  }
  .dot {
    @apply w-2 h-2 rounded-full bg-zinc-600 flex-none;
  }
  .dot.online {
    @apply bg-emerald-400;
  }
  .name {
    @apply flex-1 min-w-0 truncate text-zinc-200;
  }
  .actions {
    @apply flex gap-1 flex-none;
  }
  .add-btn {
    @apply text-[11px] px-1.5 py-0.5 rounded bg-zinc-700/60 text-zinc-200 flex-none whitespace-nowrap;
    @apply hover:bg-indigo-600 hover:text-white disabled:opacity-40 disabled:hover:bg-zinc-700/60;
  }
  .remove-btn {
    @apply hover:bg-red-600 disabled:hover:bg-zinc-700/60;
  }
  .interact-btn {
    @apply bg-amber-900/40 text-amber-200 hover:bg-amber-600 hover:text-white;
    @apply disabled:hover:bg-amber-900/40;
  }
  .icon-btn {
    @apply text-xs w-5 h-5 flex items-center justify-center rounded flex-none;
    @apply bg-zinc-700/60 hover:bg-zinc-600 disabled:opacity-40 disabled:hover:bg-zinc-700/60;
  }
  .muted {
    @apply px-3 py-1 text-xs text-zinc-500;
  }
  .new-agent {
    @apply flex items-center justify-center gap-1.5 mx-2 my-2 px-2 py-1.5 rounded-md text-xs text-zinc-300;
    @apply border border-dashed border-zinc-600 hover:bg-white/5 disabled:opacity-40;
  }
  .form {
    @apply flex flex-col gap-1.5 p-2 border-t border-zinc-700/60;
  }
  .field {
    @apply px-2 py-1 rounded bg-zinc-800 text-sm text-zinc-100 outline-none ring-1 ring-zinc-700 focus:ring-indigo-500;
  }
  .form-actions {
    @apply flex justify-end gap-1.5 mt-0.5;
  }
  .btn {
    @apply px-2 py-1 rounded text-xs text-zinc-300 hover:bg-white/10;
  }
  .btn-primary {
    @apply bg-indigo-600 text-white hover:bg-indigo-500 disabled:opacity-40 disabled:hover:bg-indigo-600;
  }
</style>
