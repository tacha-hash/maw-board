<!--
  Device pairing approval (VR5 F1d). The native connector app opens
  /pair?code=<user_code> in the browser; the logged-in user confirms the device
  name and approves, which binds the pairing to their account and hands the app
  a connector token (server-side, via /api/pair/approve). Login-gated like
  /account. Backend endpoints are Le's (deployed).
-->
<script lang="ts">
  import { onMount } from "svelte";

  type State = "loading" | "needs-login" | "invalid" | "ready" | "approving" | "approved" | "error";
  let state: State = "loading";
  let deviceName = "";
  let errorMsg = "";
  let code = "";

  onMount(async () => {
    code = new URLSearchParams(window.location.search).get("code") ?? "";
    if (!code) {
      state = "invalid";
      return;
    }
    try {
      const res = await fetch(`/api/pair/lookup?code=${encodeURIComponent(code)}`);
      if (res.status === 401) {
        // Not logged in — bounce to login, returning here afterward.
        window.location.href = `/login?next=${encodeURIComponent(`/pair?code=${code}`)}`;
        return;
      }
      if (res.status === 404) {
        state = "invalid";
        return;
      }
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      deviceName = (await res.json()).device_name ?? "this device";
      state = "ready";
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : "Something went wrong";
      state = "error";
    }
  });

  async function approve() {
    state = "approving";
    try {
      const res = await fetch("/api/pair/approve", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ user_code: code }),
      });
      if (res.status === 404) {
        state = "invalid"; // expired between lookup and approve
        return;
      }
      if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
      state = "approved";
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : "Couldn't approve";
      state = "error";
    }
  }
</script>

<svelte:head><title>Pair a device — Oracle Board</title></svelte:head>

<main class="pair">
  <div class="card panel">
    <h1>Connect a device</h1>

    {#if state === "loading"}
      <p class="muted">Checking the pairing code…</p>
    {:else if state === "invalid"}
      <p class="err">This pairing code is invalid or has expired.</p>
      <p class="muted">Open the link from your connector app again to get a fresh code.</p>
    {:else if state === "error"}
      <p class="err">{errorMsg}</p>
      <button class="btn" on:click={() => location.reload()}>Try again</button>
    {:else if state === "approved"}
      <p class="ok">🟢 Connected! You can return to the app now.</p>
      <p class="muted">Your agents will appear on your boards shortly.</p>
    {:else if state === "ready" || state === "approving"}
      <p class="ask">
        Allow the device <strong>“{deviceName}”</strong> to run your agents and serve your boards?
      </p>
      <p class="muted">Only approve a device you just started the connector app on.</p>
      <div class="actions">
        <a class="btn" href="/account">Cancel</a>
        <button class="btn btn-primary" disabled={state === "approving"} on:click={approve}>
          {state === "approving" ? "Approving…" : "Approve"}
        </button>
      </div>
    {/if}
  </div>
</main>

<style lang="postcss">
  .pair {
    @apply max-w-md mx-auto p-8 text-zinc-200;
  }
  .card {
    @apply p-6 flex flex-col gap-3;
  }
  h1 {
    @apply text-xl font-semibold;
  }
  .muted {
    @apply text-sm text-zinc-500;
  }
  .ask {
    @apply text-base text-zinc-100;
  }
  .err {
    @apply text-sm text-red-400;
  }
  .ok {
    @apply text-lg font-medium text-emerald-300;
  }
  .actions {
    @apply flex gap-2 justify-end mt-2;
  }
  .btn {
    @apply px-4 py-2 rounded-md text-sm text-zinc-300 hover:bg-white/10;
  }
  .btn-primary {
    @apply bg-indigo-600 text-white font-medium hover:bg-indigo-500 disabled:opacity-40 disabled:hover:bg-indigo-600;
  }
</style>
