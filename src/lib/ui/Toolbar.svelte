<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import {
    ImageIcon,
    MessageSquareIcon,
    MicIcon,
    MonitorIcon,
    PlusCircleIcon,
    SettingsIcon,
    TerminalIcon,
    WifiIcon,
  } from "svelte-feather-icons";

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
    <div class="brand">
      <TerminalIcon size="18" strokeWidth={2} />
      <span>Oracle Board</span>
    </div>

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
        on:click={() => dispatch("micDown")}
        disabled={!connected}
        title={micRecording ? "Mute mic" : "Unmute mic"}
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
  .brand {
    @apply flex items-center gap-1.5 text-indigo-400 font-semibold text-sm tracking-tight;
  }

  .v-divider {
    @apply h-5 mx-2 border-l border-zinc-700;
  }

  .icon-button {
    @apply relative rounded-lg p-1.5 text-zinc-400 hover:text-white hover:bg-zinc-700/80;
    @apply active:bg-indigo-600 transition-all duration-150;
    @apply disabled:opacity-30 disabled:bg-transparent disabled:text-zinc-600;
  }

  .icon-button.recording {
    @apply bg-red-600 text-white hover:bg-red-500;
  }

  .icon-button.active {
    @apply bg-indigo-600 text-white hover:bg-indigo-500;
  }

  .activity {
    @apply absolute top-0.5 right-0.5 p-[4px] bg-red-500 rounded-full;
  }
</style>
