<!--
  Lobby — replaces the original sshx CLI-install landing page (the fork is
  fully independent now, PLAN.md 2026-07-06 ค่ำ; that content had no audience
  here). "Board = Project" (PRODUCT SPEC point 5) makes this list the natural
  root/home experience, not a secondary /lobby path — see
  docs/lobby-ui-design.md (le-workboard) for the full reasoning, including
  the localStorage key-recovery approach `GET /api/boards` architecturally
  can't help with (the server never sees encryption keys at all).
-->
<script lang="ts">
  import { onMount } from "svelte";
  import {
    PlusIcon,
    RefreshCwIcon,
    FolderIcon,
    ClockIcon,
    KeyIcon,
  } from "svelte-feather-icons";
  import { makeToast } from "$lib/toast";

  type BoardInfo = { name: string; live: boolean; modified: number | null; size: number | null };

  let boards: BoardInfo[] = [];
  let loading = true;
  let loadError = "";

  const KEY_STORAGE = "oracle-board-keys";

  function loadKeyMap(): Record<string, string> {
    try {
      return JSON.parse(localStorage.getItem(KEY_STORAGE) ?? "{}");
    } catch {
      return {};
    }
  }

  function saveKey(name: string, key: string) {
    const map = loadKeyMap();
    map[name] = key;
    localStorage.setItem(KEY_STORAGE, JSON.stringify(map));
  }

  async function loadBoards() {
    loading = true;
    loadError = "";
    try {
      const res = await fetch("/api/boards");
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      // Newest/live first — most likely what you want to jump back into.
      boards = (data as BoardInfo[]).sort((a, b) => {
        if (a.live !== b.live) return a.live ? -1 : 1;
        return (b.modified ?? 0) - (a.modified ?? 0);
      });
    } catch (e) {
      loadError = e instanceof Error ? e.message : "failed to load";
    } finally {
      loading = false;
    }
  }

  onMount(loadBoards);

  function relativeTime(unixSecs: number | null, live: boolean): string {
    // `modified` is null for a live session with no disk snapshot yet — for
    // a non-live board this shouldn't really happen, but "active now" would
    // be actively misleading in that case, so don't assume liveness from it.
    if (unixSecs === null) return live ? "active now" : "no activity recorded";
    const diffMs = Date.now() - unixSecs * 1000;
    const mins = Math.floor(diffMs / 60000);
    if (mins < 1) return "just now";
    if (mins < 60) return `${mins}m ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ago`;
    return `${Math.floor(hours / 24)}d ago`;
  }

  // "Open" needs the board's encryption key, which the server never has
  // (E2E — keys live only in the URL fragment). Boards this lobby created
  // have it in localStorage already; anything else (a board from before
  // the lobby existed, or opened from a different browser) needs a manual
  // paste — see docs/lobby-ui-design.md.
  let pendingOpenName: string | null = null;
  let manualKeyInput = "";

  function keyFor(name: string): string | undefined {
    return loadKeyMap()[name];
  }

  function openWithKey(name: string, key: string) {
    window.location.href = `${window.location.origin}/s/${name}#${key}`;
  }

  function requestManualKey(name: string) {
    pendingOpenName = name;
    manualKeyInput = "";
  }

  function submitManualKey() {
    if (!pendingOpenName || !manualKeyInput.trim()) return;
    let key = manualKeyInput.trim();
    const hashIdx = key.indexOf("#");
    if (hashIdx >= 0) key = key.slice(hashIdx + 1);
    saveKey(pendingOpenName, key);
    openWithKey(pendingOpenName, key);
  }

  let showNewForm = false;
  let newBoardName = "";
  let creating = false;

  async function createBoard() {
    creating = true;
    try {
      const res = await fetch("/api/boards/new", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(newBoardName.trim() ? { name: newBoardName.trim() } : {}),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const { name, key } = await res.json();
      saveKey(name, key);
      openWithKey(name, key);
    } catch (e) {
      makeToast({
        kind: "error",
        message: `Couldn't create board: ${e instanceof Error ? e.message : "unknown error"}`,
      });
    } finally {
      creating = false;
    }
  }
</script>

<svelte:head>
  <title>Oracle Board</title>
</svelte:head>

<!-- <svelte:window> must be at the template's top level, not nested inside
     an {#if} — Escape only matters while the modal is open, guarded here. -->
<svelte:window
  on:keydown={(e) => e.key === "Escape" && pendingOpenName && (pendingOpenName = null)}
/>

<main class="lobby">
  <div class="lobby-head">
    <h1>Oracle Board</h1>
    <div class="head-actions">
      <button class="icon-btn" title="Refresh" on:click={loadBoards} disabled={loading}>
        <RefreshCwIcon size="16" class={loading ? "spin" : ""} />
      </button>
      <button class="new-board-btn" on:click={() => (showNewForm = !showNewForm)}>
        <PlusIcon size="16" />
        New Board
      </button>
    </div>
  </div>

  {#if showNewForm}
    <div class="new-form panel">
      <input
        class="field"
        placeholder="Board name (optional — random if left blank)"
        bind:value={newBoardName}
        on:keydown={(e) => e.key === "Enter" && createBoard()}
      />
      <button class="btn" on:click={() => (showNewForm = false)}>Cancel</button>
      <button class="btn btn-primary" disabled={creating} on:click={createBoard}>
        {creating ? "Creating…" : "Create"}
      </button>
    </div>
  {/if}

  {#if loading}
    <p class="muted">Loading boards…</p>
  {:else if loadError}
    <p class="error-text">Couldn't load boards: {loadError}</p>
  {:else if boards.length === 0}
    <p class="muted">No boards yet — create one to get started.</p>
  {:else}
    <div class="board-list">
      {#each boards as board (board.name)}
        <div class="board-row panel">
          <span class="dot" class:live={board.live} title={board.live ? "Live" : "Sleeping"} />
          <span class="board-name">{board.name}</span>
          <span class="board-meta">
            <ClockIcon size="12" />
            {relativeTime(board.modified, board.live)}
          </span>
          {#if keyFor(board.name)}
            <button class="open-btn" on:click={() => openWithKey(board.name, keyFor(board.name) ?? "")}>
              <FolderIcon size="14" />
              Open
            </button>
          {:else}
            <button class="open-btn open-btn-secondary" on:click={() => requestManualKey(board.name)}>
              <KeyIcon size="14" />
              Need key
            </button>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  {#if pendingOpenName}
    <div
      class="modal-backdrop"
      role="button"
      tabindex="0"
      on:click={() => (pendingOpenName = null)}
      on:keydown={(e) => e.key === "Enter" && (pendingOpenName = null)}
    >
      <!-- svelte-ignore a11y-click-events-have-key-events -->
      <div class="modal panel" on:click={(e) => e.stopPropagation()}>
        <p>No key remembered on this device for <strong>{pendingOpenName}</strong>.</p>
        <p class="muted">Paste the full board URL, or just the key after the #.</p>
        <input
          class="field"
          placeholder="https://.../s/name#key or just the key"
          bind:value={manualKeyInput}
          on:keydown={(e) => e.key === "Enter" && submitManualKey()}
        />
        <div class="modal-actions">
          <button class="btn" on:click={() => (pendingOpenName = null)}>Cancel</button>
          <button class="btn btn-primary" disabled={!manualKeyInput.trim()} on:click={submitManualKey}>
            Open
          </button>
        </div>
      </div>
    </div>
  {/if}
</main>

<style lang="postcss">
  .lobby {
    @apply max-w-2xl mx-auto p-8 text-zinc-200;
  }
  .lobby-head {
    @apply flex items-center justify-between mb-6;
  }
  h1 {
    @apply text-2xl font-semibold;
  }
  .head-actions {
    @apply flex items-center gap-2;
  }
  .icon-btn {
    @apply p-2 rounded-md bg-zinc-800 text-zinc-300 hover:bg-zinc-700 disabled:opacity-50;
  }
  :global(.icon-btn .spin) {
    @apply animate-spin;
  }
  .new-board-btn {
    @apply flex items-center gap-1.5 px-3 py-2 rounded-md bg-indigo-600 text-white text-sm font-medium hover:bg-indigo-500;
  }
  .new-form {
    @apply flex gap-2 p-3 mb-4;
  }
  .field {
    @apply flex-1 px-3 py-1.5 rounded bg-zinc-800 text-sm text-zinc-100 outline-none ring-1 ring-zinc-700 focus:ring-indigo-500;
  }
  .btn {
    @apply px-3 py-1.5 rounded text-sm text-zinc-300 hover:bg-white/10;
  }
  .btn-primary {
    @apply bg-indigo-600 text-white hover:bg-indigo-500 disabled:opacity-40 disabled:hover:bg-indigo-600;
  }
  .muted {
    @apply text-zinc-500 text-sm;
  }
  .error-text {
    @apply text-red-400 text-sm;
  }
  .board-list {
    @apply flex flex-col gap-2;
  }
  .board-row {
    @apply flex items-center gap-3 px-4 py-3;
  }
  .dot {
    @apply w-2.5 h-2.5 rounded-full bg-zinc-600 flex-none;
  }
  .dot.live {
    @apply bg-emerald-400;
  }
  .board-name {
    @apply flex-1 min-w-0 truncate font-medium;
  }
  .board-meta {
    @apply flex items-center gap-1 text-xs text-zinc-500 flex-none;
  }
  .open-btn {
    @apply flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-zinc-700/80 text-zinc-100 text-sm;
    @apply hover:bg-indigo-600 flex-none;
  }
  .open-btn-secondary {
    @apply bg-zinc-800 text-zinc-400 hover:bg-amber-600 hover:text-white;
  }
  .modal-backdrop {
    @apply fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4;
  }
  .modal {
    @apply flex flex-col gap-2 p-5 max-w-md w-full;
  }
  .modal-actions {
    @apply flex justify-end gap-2 mt-1;
  }
</style>
