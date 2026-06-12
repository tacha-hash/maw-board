<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import {
    ImageIcon,
    MessageSquareIcon,
    MicIcon,
    MonitorIcon,
    PlusCircleIcon,
    SettingsIcon,
    WifiIcon,
  } from "svelte-feather-icons";

  import logo from "$lib/assets/logo.svg";

  export let connected: boolean;
  export let hasWriteAccess: boolean | undefined;
  export let newMessages: boolean;
  // ── maw share workboard extensions ──
  export let micRecording = false;
  export let streamActive = false;

  const dispatch = createEventDispatcher<{
    create: void;
    chat: void;
    settings: void;
    networkInfo: void;
    micDown: void;
    image: void;
    stream: void;
  }>();
</script>

<div class="panel inline-block px-3 py-2">
  <div class="flex items-center select-none">
    <a href="/" class="flex-shrink-0"
      ><img src={logo} alt="sshx logo" class="h-10" /></a
    >
    <p class="ml-1.5 mr-2 font-medium">sshx</p>

    <div class="v-divider" />

    <div class="flex space-x-1">
      <button
        class="icon-button"
        on:click={() => dispatch("create")}
        disabled={!connected || !hasWriteAccess}
        title={!connected
          ? "Not connected"
          : hasWriteAccess === false // Only show the "No write access" title after confirming read-only mode.
          ? "No write access"
          : "Create new terminal"}
      >
        <PlusCircleIcon strokeWidth={1.5} class="p-0.5" />
      </button>
      <button class="icon-button" on:click={() => dispatch("chat")}>
        <MessageSquareIcon strokeWidth={1.5} class="p-0.5" />
        {#if newMessages}
          <div class="activity" />
        {/if}
      </button>
      <button class="icon-button" on:click={() => dispatch("settings")}>
        <SettingsIcon strokeWidth={1.5} class="p-0.5" />
      </button>
    </div>

    <div class="v-divider" />

    <!-- maw share workboard extensions: voice, image, screen-share -->
    <div class="flex space-x-1">
      <button
        class="icon-button"
        class:recording={micRecording}
        on:pointerdown={() => dispatch("micDown")}
        disabled={!connected}
        title="Hold to talk"
      >
        <MicIcon strokeWidth={1.5} class="p-0.5" />
      </button>
      <button
        class="icon-button"
        on:click={() => dispatch("image")}
        disabled={!connected || hasWriteAccess === false}
        title="Add image to board"
      >
        <ImageIcon strokeWidth={1.5} class="p-0.5" />
      </button>
      <button
        class="icon-button"
        class:active={streamActive}
        on:click={() => dispatch("stream")}
        disabled={!connected || hasWriteAccess === false}
        title={streamActive ? "Stop screen share" : "Share screen"}
      >
        <MonitorIcon strokeWidth={1.5} class="p-0.5" />
      </button>
    </div>

    <div class="v-divider" />

    <div class="flex space-x-1">
      <button class="icon-button" on:click={() => dispatch("networkInfo")}>
        <WifiIcon strokeWidth={1.5} class="p-0.5" />
      </button>
    </div>
  </div>
</div>

<style lang="postcss">
  .v-divider {
    @apply h-5 mx-2 border-l-4 border-zinc-800;
  }

  .icon-button {
    @apply relative rounded-md p-1 hover:bg-zinc-700 active:bg-indigo-700 transition-colors;
    @apply disabled:opacity-50 disabled:bg-transparent;
  }

  .icon-button.recording {
    @apply bg-red-600 text-white hover:bg-red-600;
  }

  .icon-button.active {
    @apply bg-indigo-600 text-white hover:bg-indigo-600;
  }

  .activity {
    @apply absolute top-1 right-0.5 text-xs p-[4.5px] bg-red-500 rounded-full;
  }
</style>
