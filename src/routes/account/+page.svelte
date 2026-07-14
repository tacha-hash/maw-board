<!--
  Account settings (VR5 F0.5). Home for per-account controls: change password
  (B1, here) and — coming in B2 — the connector section (token rotate + Download
  + live online status via /api/connector/status), where the onboarding wizard
  plugs in. Reached at /account; the API calls require a session cookie.
-->
<script lang="ts">
  import { makeToast } from "$lib/toast";

  let oldPassword = "";
  let newPassword = "";
  let confirmPassword = "";
  let submitting = false;

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
</style>
