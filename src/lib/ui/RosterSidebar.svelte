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
  // Session.svelte measures the online-users NameList's real height (it
  // sits right above this panel in the same left column) and passes the
  // pixel offset to start below it — a fixed CSS top overlapped it
  // whenever enough people were connected (Louis hit this live).
  export let topPx = 80;
  // VR5 F0.5 wizard: is this account's connector live on the control channel?
  // Adding an agent needs one running, so "New agent" is gated on it. Defaults
  // true so a legacy/non-account board never blocks.
  export let connectorOnline = true;

  const MODELS = ["sonnet", "opus", "haiku"];

  const dispatch = createEventDispatcher<{
    close: void;
    addAgent: { name: string; mode: "view" | "interact" };
    removeAgent: string;
    createAgent: {
      name: string;
      host: string;
      folder: string;
      hasRepo: boolean;
      repoUrl: string;
      createRepo: boolean;
      model: string;
    };
  }>();

  let showForm = false;
  let formName = "";
  let formHost = "";
  let formFolder = "";
  let formHasRepo = true;
  let formRepoUrl = "";
  let formCreateRepo = false;
  let formModel = MODELS[0];

  function resetForm() {
    formName = "";
    formHost = "";
    formFolder = "";
    formHasRepo = true;
    formRepoUrl = "";
    formCreateRepo = false;
    formModel = MODELS[0];
    showForm = false;
  }

  function submitForm() {
    if (!formName.trim()) return;
    dispatch("createAgent", {
      name: formName.trim(),
      host: formHost.trim(),
      folder: formFolder.trim(),
      hasRepo: formHasRepo,
      repoUrl: formHasRepo ? formRepoUrl.trim() : "",
      createRepo: !formHasRepo && formCreateRepo,
      model: formModel,
    });
    resetForm();
  }
</script>

<div class="roster panel" style:top="{topPx}px">
  <div class="head">
    <div class="flex items-center gap-1.5 text-zinc-200 font-medium text-sm">
      <UsersIcon size="15" />
      <span>Agents</span>
    </div>
    <button class="close" title="Close" on:click={() => dispatch("close")}>
      <XIcon size="15" />
    </button>
  </div>

  {#if !connectorOnline}
    <div class="offline">
      <span class="dot-off"></span>
      No connector online. <a class="link" href="/account">Set one up</a> to run your agents.
    </div>
  {/if}

  <div class="list">
    {#if agents.length === 0}
      <p class="muted">
        {#if connectorOnline}Waiting for roster from bridge…{:else}No agents yet.{/if}
      </p>
    {/if}
    {#each agents as agent (agent.name)}
      {@const mirrored = onBoard.has(agent.name)}
      {@const sleeping = agent.status === "sleeping"}
      <div class="row" class:sleeping>
        <span class="dot" class:online={mirrored} />
        <span
          class="name"
          title={sleeping ? "Sleeping — Add wakes it up" : agent.host ?? ""}
        >
          {agent.name}{#if sleeping}<span class="sleep-icon">💤</span>{/if}
        </span>
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
    <!-- New Agent Wizard (PLAN.md — new agent creation always needs a
         supervisor per fleet practice; this only posts kind:"agent-create"
         for Le to review, it never spawns anything itself). -->
    <div class="form">
      <input class="field" placeholder="Agent name" bind:value={formName} />
      <input class="field" placeholder="Host (e.g. pc, mac)" bind:value={formHost} />
      <input class="field" placeholder="Folder (working directory)" bind:value={formFolder} />

      <div class="form-toggle">
        <button
          class="toggle-btn"
          class:selected={formHasRepo}
          on:click={() => (formHasRepo = true)}
        >
          Has a repo
        </button>
        <button
          class="toggle-btn"
          class:selected={!formHasRepo}
          on:click={() => (formHasRepo = false)}
        >
          No repo yet
        </button>
      </div>

      {#if formHasRepo}
        <input class="field" placeholder="Repo URL" bind:value={formRepoUrl} />
      {:else}
        <label class="checkbox-row">
          <input type="checkbox" bind:checked={formCreateRepo} />
          <span>Create a new repo for it</span>
        </label>
      {/if}

      <div class="field-label">Model</div>
      <div class="form-toggle">
        {#each MODELS as m}
          <button
            class="toggle-btn"
            class:selected={formModel === m}
            on:click={() => (formModel = m)}
          >
            {m}
          </button>
        {/each}
      </div>

      <div class="form-actions">
        <button class="btn" on:click={resetForm}>Cancel</button>
        <button class="btn btn-primary" disabled={!formName.trim()} on:click={submitForm}>
          Submit for review
        </button>
      </div>
    </div>
  {:else}
    <button
      class="new-agent"
      disabled={!canEdit || !connectorOnline}
      title={!connectorOnline ? "Start your connector first (Account settings)" : undefined}
      on:click={() => (showForm = true)}
    >
      <PlusIcon size="14" />
      New agent
    </button>
  {/if}
</div>

<style lang="postcss">
  /* top-36 (not top-20, like FileExplorer) — the "online users" NameList
     (Session.svelte, `fixed left-3 top-24`) occupies the same top-left
     corner and Roster defaults open (unlike FileExplorer) — `top` comes in
     as `topPx` (measured live from NameList's actual height) rather than a
     fixed class, since a fixed guess overlapped it once enough people were
     connected to make NameList taller than expected. */
  .roster {
    @apply fixed left-2 right-2 sm:right-auto sm:left-4 bottom-4 w-auto sm:w-72 z-30 flex flex-col p-0 overflow-hidden;
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
  .row.sleeping .name {
    @apply text-zinc-500;
  }
  .row.sleeping .dot {
    @apply bg-zinc-700;
  }
  .sleep-icon {
    @apply ml-1 text-[10px];
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
  .offline {
    @apply flex items-center gap-1.5 mx-2 mt-2 px-2.5 py-1.5 rounded-md text-[11px] text-amber-200/90 bg-amber-500/10 ring-1 ring-amber-500/25;
  }
  .dot-off {
    @apply w-2 h-2 rounded-full bg-amber-400 flex-none;
  }
  .link {
    @apply underline text-amber-200 hover:text-amber-100;
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
  .field-label {
    @apply text-[10px] uppercase tracking-wide text-zinc-500 mt-0.5;
  }
  .form-toggle {
    @apply flex gap-1;
  }
  .toggle-btn {
    @apply flex-1 text-xs px-1.5 py-1 rounded bg-zinc-800 text-zinc-300 ring-1 ring-zinc-700;
    @apply hover:ring-indigo-500;
  }
  .toggle-btn.selected {
    @apply bg-indigo-500/20 ring-indigo-500 text-indigo-200;
  }
  .checkbox-row {
    @apply flex items-center gap-1.5 text-xs text-zinc-300 px-0.5;
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
