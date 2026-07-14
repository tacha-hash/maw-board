<!--
  Members & permissions panel (VR5 F0.5 B3). Shows everyone on the board with
  their capability level; the board OWNER can share to a username, set a member's
  level (view / edit / order), and remove them. Members see the roster read-only.
  "order" implies "edit" (the server normalizes it). The type-into-terminal
  invariant (owner-only) is unrelated to these flags.
-->
<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { XIcon, Trash2Icon, UserPlusIcon } from "svelte-feather-icons";
  import { makeToast } from "$lib/toast";

  type Member = { username: string; role: string; can_edit: boolean; can_order: boolean };
  type Level = "view" | "edit" | "order";

  export let members: Member[] = [];
  export let isOwner = false;
  export let myUsername = "";
  export let boardName: string;

  const dispatch = createEventDispatcher<{ changed: void; close: void }>();
  let newUsername = "";
  let busy = false;

  const LEVELS: Level[] = ["view", "edit", "order"];
  const levelOf = (m: Member): Level =>
    m.role === "owner" ? "edit" : m.can_order ? "order" : m.can_edit ? "edit" : "view";

  const api = (path: string) => `/api/boards/${encodeURIComponent(boardName)}${path}`;

  async function req(url: string, method: string, body?: unknown): Promise<boolean> {
    busy = true;
    try {
      const res = await fetch(url, {
        method,
        headers: body ? { "Content-Type": "application/json" } : undefined,
        body: body ? JSON.stringify(body) : undefined,
      });
      if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
      return true;
    } catch (e) {
      makeToast({ kind: "error", message: e instanceof Error ? e.message : "Request failed" });
      return false;
    } finally {
      busy = false;
    }
  }

  async function setLevel(m: Member, level: Level) {
    if (levelOf(m) === level) return;
    const can_edit = level === "edit" || level === "order";
    const can_order = level === "order";
    if (await req(api(`/members/${encodeURIComponent(m.username)}`), "PATCH", { can_edit, can_order }))
      dispatch("changed");
  }

  async function addMember() {
    const u = newUsername.trim();
    if (!u) return;
    if (await req(api("/members"), "POST", { username: u })) {
      newUsername = "";
      makeToast({ kind: "success", message: `Shared with ${u}` });
      dispatch("changed");
    }
  }

  async function removeMember(m: Member) {
    if (!window.confirm(`Remove ${m.username} from this board?`)) return;
    if (await req(api(`/members/${encodeURIComponent(m.username)}`), "DELETE")) dispatch("changed");
  }
</script>

<div class="members panel">
  <div class="head">
    <span class="title">Members &amp; permissions</span>
    <button class="x" aria-label="Close" on:click={() => dispatch("close")}><XIcon size="16" /></button>
  </div>

  <div class="list">
    {#each members as m (m.username)}
      <div class="row">
        <span class="name" title={m.username}>
          {m.username}{m.username === myUsername ? " (you)" : ""}
        </span>
        {#if m.role === "owner"}
          <span class="badge owner">Owner</span>
        {:else if isOwner}
          <div class="levels">
            {#each LEVELS as lvl}
              <button
                class="lvl"
                class:sel={levelOf(m) === lvl}
                disabled={busy}
                title={lvl === "view" ? "Read-only" : lvl === "edit" ? "Can edit the board" : "Can dispatch Work Orders"}
                on:click={() => setLevel(m, lvl)}
              >
                {lvl}
              </button>
            {/each}
            <button class="rm" title="Remove from board" disabled={busy} on:click={() => removeMember(m)}>
              <Trash2Icon size="14" />
            </button>
          </div>
        {:else}
          <span class="badge">{levelOf(m)}</span>
        {/if}
      </div>
    {/each}
  </div>

  {#if isOwner}
    <form class="add" on:submit|preventDefault={addMember}>
      <input placeholder="Share with a username…" bind:value={newUsername} disabled={busy} />
      <button type="submit" title="Share" disabled={busy || !newUsername.trim()}>
        <UserPlusIcon size="15" />
      </button>
    </form>
    <p class="hint">Only the owner can type into terminals — that never changes.</p>
  {/if}
</div>

<style lang="postcss">
  .members {
    @apply fixed right-3 top-24 z-30 flex flex-col gap-2 p-3 w-[300px] max-h-[70vh] overflow-y-auto;
  }
  .head {
    @apply flex items-center justify-between pb-1.5 border-b border-white/10;
  }
  .title {
    @apply text-sm font-semibold text-zinc-100;
  }
  .x {
    @apply p-0.5 rounded text-zinc-400 hover:bg-white/10 hover:text-zinc-100;
  }
  .list {
    @apply flex flex-col gap-1.5;
  }
  .row {
    @apply flex items-center justify-between gap-2;
  }
  .name {
    @apply text-sm text-zinc-200 truncate min-w-0;
  }
  .badge {
    @apply px-1.5 py-0.5 rounded text-[10px] font-medium bg-white/10 text-zinc-300 flex-none;
  }
  .badge.owner {
    @apply bg-amber-500/20 text-amber-300;
  }
  .levels {
    @apply flex items-center gap-0.5 flex-none;
  }
  .lvl {
    @apply px-1.5 py-0.5 rounded text-[11px] text-zinc-400 hover:bg-white/10 disabled:opacity-40;
  }
  .lvl.sel {
    @apply bg-indigo-600 text-white;
  }
  .rm {
    @apply ml-1 p-1 rounded text-zinc-500 hover:bg-red-500/15 hover:text-red-300 disabled:opacity-40;
  }
  .add {
    @apply flex gap-1.5 pt-1;
  }
  .add input {
    @apply flex-1 min-w-0 px-2 py-1.5 rounded bg-zinc-800 text-sm text-zinc-100 outline-none ring-1 ring-zinc-700 focus:ring-indigo-500;
  }
  .add button {
    @apply px-2.5 rounded bg-indigo-600 text-white hover:bg-indigo-500 disabled:opacity-40 flex-none;
  }
  .hint {
    @apply text-[10px] text-zinc-500;
  }
</style>
