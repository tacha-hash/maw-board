<!--
  Account settings (VR5 F0.5). Home for per-account controls: change password
  (B1, here) and — coming in B2 — the connector section (token rotate + Download
  + live online status via /api/connector/status), where the onboarding wizard
  plugs in. Reached at /account; the API calls require a session cookie.
-->
<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { makeToast } from "$lib/toast";

  let oldPassword = "";
  let newPassword = "";
  let confirmPassword = "";
  let submitting = false;

  // ── Connector section (VR5 F0.5 B2) ──────────────────────────────────────
  // Live online status (control-channel-backed /api/connector/status) + token
  // configured state + rotate. The onboarding wizard (Add-agent gate) plugs in
  // on top of this status in a later pass.
  let online = false;
  let connectors = 0;
  let statusKnown = false;
  let tokenConfigured = false;
  let rotating = false;
  let rawToken = ""; // shown once, right after a rotate
  let poll: ReturnType<typeof setInterval> | undefined;

  async function refreshStatus() {
    try {
      const res = await fetch("/api/connector/status");
      if (res.ok) {
        const j = await res.json();
        online = !!j.online;
        connectors = j.connectors ?? 0;
        statusKnown = true;
      }
    } catch {
      /* transient — keep last known status */
    }
  }

  async function refreshToken() {
    try {
      const res = await fetch("/api/account/connector-token");
      if (res.ok) tokenConfigured = !!(await res.json()).configured;
    } catch {
      /* ignore */
    }
  }

  async function rotateToken() {
    if (tokenConfigured && !confirm("Generate a new token? Your current connector token stops working immediately.")) {
      return;
    }
    rotating = true;
    try {
      const res = await fetch("/api/account/connector-token/rotate", { method: "POST" });
      if (!res.ok) {
        makeToast({ kind: "error", message: (await res.text()) || `Error ${res.status}` });
        return;
      }
      rawToken = (await res.json()).token;
      tokenConfigured = true;
      makeToast({ kind: "success", message: "New connector token generated — copy it now." });
    } catch {
      makeToast({ kind: "error", message: "Network error — please try again." });
    } finally {
      rotating = false;
    }
  }

  async function copyToken() {
    try {
      await navigator.clipboard.writeText(rawToken);
      makeToast({ kind: "success", message: "Token copied to clipboard." });
    } catch {
      makeToast({ kind: "info", message: "Select the token and copy it manually." });
    }
  }

  onMount(() => {
    refreshStatus();
    refreshToken();
    // Poll so a just-started connector flips to online within a few seconds
    // (live pairing feedback over the control channel).
    poll = setInterval(refreshStatus, 3000);
  });
  onDestroy(() => clearInterval(poll));

  async function changePassword() {
    if (newPassword !== confirmPassword) {
      makeToast({ kind: "error", message: "New passwords don't match." });
      return;
    }
    if (newPassword.length < 8) {
      makeToast({ kind: "error", message: "New password must be at least 8 characters." });
      return;
    }
    submitting = true;
    try {
      const res = await fetch("/api/account/password", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ old_password: oldPassword, new_password: newPassword }),
      });
      if (!res.ok) {
        // Server messages are human-readable ("current password is incorrect",
        // "login required", the length rule, …).
        makeToast({ kind: "error", message: (await res.text()) || `Error ${res.status}` });
        return;
      }
      oldPassword = newPassword = confirmPassword = "";
      makeToast({ kind: "success", message: "Password changed — every other session was signed out." });
    } catch {
      makeToast({ kind: "error", message: "Network error — please try again." });
    } finally {
      submitting = false;
    }
  }
</script>

<svelte:head><title>Account — Oracle Board</title></svelte:head>

<main class="account">
  <div class="head">
    <a class="back" href="/">← Boards</a>
    <h1>Account settings</h1>
  </div>

  <section class="card panel">
    <h2>Change password</h2>
    <p class="muted">Changing your password signs you out of every other device.</p>
    <form on:submit|preventDefault={changePassword}>
      <label>
        Current password
        <input type="password" bind:value={oldPassword} autocomplete="current-password" required />
      </label>
      <label>
        New password
        <input type="password" bind:value={newPassword} autocomplete="new-password" minlength={8} required />
      </label>
      <label>
        Confirm new password
        <input type="password" bind:value={confirmPassword} autocomplete="new-password" required />
      </label>
      <button class="btn-primary" type="submit" disabled={submitting}>
        {submitting ? "Saving…" : "Change password"}
      </button>
    </form>
  </section>

  <section class="card panel">
    <h2>Connector</h2>
    <p class="muted">Your connector runs your agents and serves your boards to this app.</p>

    <div class="status" class:online>
      <span class="dot"></span>
      {#if !statusKnown}
        Checking connector status…
      {:else if online}
        Connector online{connectors > 1 ? ` — ${connectors} running` : ""}
      {:else}
        No connector online — generate a token and start one below.
      {/if}
    </div>

    <div class="token">
      <span class="muted">Connector token: {tokenConfigured ? "configured" : "not set up yet"}</span>
      <button class="btn" on:click={rotateToken} disabled={rotating}>
        {rotating ? "Generating…" : tokenConfigured ? "Generate new token" : "Generate token"}
      </button>
    </div>

    {#if rawToken}
      <div class="raw">
        <p class="warn">Copy this now — it's shown only once and can't be recovered.</p>
        <div class="raw-row">
          <code>{rawToken}</code>
          <button class="btn" on:click={copyToken}>Copy</button>
        </div>
        <p class="muted small">
          Set it as <code>CONNECTOR_TOKEN</code> in your connector's environment, then start it.
        </p>
      </div>
    {/if}
  </section>
</main>

<style lang="postcss">
  .account {
    @apply max-w-xl mx-auto p-8 text-zinc-200;
  }
  .head {
    @apply mb-6;
  }
  .back {
    @apply text-sm text-zinc-500 hover:text-zinc-300;
  }
  h1 {
    @apply text-2xl font-semibold mt-1;
  }
  .card {
    @apply p-5 flex flex-col gap-1.5;
  }
  h2 {
    @apply text-lg font-medium;
  }
  .muted {
    @apply text-sm text-zinc-500 mb-2;
  }
  form {
    @apply flex flex-col gap-3;
  }
  label {
    @apply flex flex-col gap-1 text-sm text-zinc-300;
  }
  input {
    @apply px-3 py-2 rounded bg-zinc-800 text-sm text-zinc-100 outline-none ring-1 ring-zinc-700 focus:ring-indigo-500;
  }
  .btn-primary {
    @apply mt-1 self-start px-4 py-2 rounded-md bg-indigo-600 text-white text-sm font-medium;
    @apply hover:bg-indigo-500 disabled:opacity-40 disabled:hover:bg-indigo-600;
  }
  .btn {
    @apply px-3 py-1.5 rounded-md bg-zinc-700/80 text-zinc-100 text-sm hover:bg-zinc-700;
    @apply disabled:opacity-40 disabled:hover:bg-zinc-700/80;
  }
  .status {
    @apply flex items-center gap-2 text-sm text-zinc-300 mt-1;
  }
  .status .dot {
    @apply w-2.5 h-2.5 rounded-full bg-zinc-600 flex-none;
  }
  .status.online .dot {
    @apply bg-emerald-400;
  }
  .token {
    @apply flex items-center justify-between gap-3 mt-3;
  }
  .raw {
    @apply flex flex-col gap-1.5 mt-3 p-3 rounded-md bg-black/30 ring-1 ring-amber-500/40;
  }
  .warn {
    @apply text-xs font-medium text-amber-300;
  }
  .raw-row {
    @apply flex items-center gap-2;
  }
  code {
    @apply px-1.5 py-0.5 rounded bg-zinc-800 text-zinc-100 text-xs font-mono break-all;
  }
  .raw-row code {
    @apply flex-1;
  }
  .small {
    @apply text-xs;
  }
</style>
