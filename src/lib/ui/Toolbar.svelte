<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import StatusBar from "./StatusBar.svelte";
  import LayoutMenu from "./LayoutMenu.svelte";
  import {
    ChevronLeftIcon,
    ChevronRightIcon,
    CrosshairIcon,
    Edit2Icon,
    FileTextIcon,
    FilmIcon,
    FolderIcon,
    GridIcon,
    LockIcon,
    UnlockIcon,
    Trash2Icon,
    ImageIcon,
    MessageSquareIcon,
    MicIcon,
    PlusCircleIcon,
    SettingsIcon,
    TerminalIcon,
    VideoIcon,
    WifiIcon,
  } from "svelte-feather-icons";

  export let connected: boolean;
  export let hasWriteAccess: boolean | undefined;
  export let newMessages: boolean;
  // ── maw share workboard extensions ──
  export let micRecording = false;
  export let cameraActive = false;
  export let boardLocked = false;
  export let lockedForMe = false;

  const dispatch = createEventDispatcher<{
    create: void;
    lock: void;
    tile: string | number;
    center: void;
    clear: void;
    note: void;
    video: void;
    files: void;
    doc: void;
    chat: void;
    settings: void;
    networkInfo: void;
    micDown: void;
    image: void;
    stream: void;
    camera: void;
  }>();

  let collapsed = false;
  let showTileMenu = false;
</script>

<div class="panel inline-block px-3 py-2">
  <div class="flex items-center select-none">
    <div class="brand">
      <TerminalIcon size="18" strokeWidth={2} />
      <span>Oracle Board</span>
    </div>

    <div class="v-divider" />

    <StatusBar />

    <div class="v-divider" />

    <button
      class="icon-button"
      on:click={() => (collapsed = !collapsed)}
      title={collapsed ? "Expand toolbar" : "Collapse toolbar"}
    >
      {#if collapsed}
        <ChevronRightIcon strokeWidth={1.5} class="p-0.5" />
      {:else}
        <ChevronLeftIcon strokeWidth={1.5} class="p-0.5" />
      {/if}
    </button>

    {#if !collapsed}
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
      <div class="relative">
        <button
          class="icon-button"
          class:active={showTileMenu}
          on:click={() => (showTileMenu = !showTileMenu)}
          disabled={!connected || hasWriteAccess === false}
          title="Arrange windows"
        >
          <GridIcon strokeWidth={1.5} class="p-0.5" />
        </button>
        {#if showTileMenu}
          <LayoutMenu
            on:select={(e) => dispatch("tile", e.detail)}
            on:close={() => (showTileMenu = false)}
          />
        {/if}
      </div>
      <button
        class="icon-button"
        on:click={() => dispatch("center")}
        title="Center / reset view"
      >
        <CrosshairIcon strokeWidth={1.5} class="p-0.5" />
      </button>
      <button
        class="icon-button"
        on:click={() => dispatch("clear")}
        disabled={!connected || hasWriteAccess === false}
        title="Clear the board (notes/images/videos)"
      >
        <Trash2Icon strokeWidth={1.5} class="p-0.5" />
      </button>
      <button
        class="icon-button"
        class:lock-on={boardLocked}
        on:click={() => dispatch("lock")}
        disabled={!connected || hasWriteAccess === false || lockedForMe}
        title={boardLocked
          ? lockedForMe
            ? "Board is locked by someone else"
            : "Board locked — click to unlock"
          : "Lock the board (others become read-only)"}
      >
        {#if boardLocked}
          <LockIcon strokeWidth={1.5} class="p-0.5" />
        {:else}
          <UnlockIcon strokeWidth={1.5} class="p-0.5" />
        {/if}
      </button>
      <button
        class="icon-button"
        on:click={() => dispatch("files")}
        title="File explorer"
      >
        <FolderIcon strokeWidth={1.5} class="p-0.5" />
      </button>
      <button
        class="icon-button"
        on:click={() => dispatch("doc")}
        title="Shared document"
      >
        <Edit2Icon strokeWidth={1.5} class="p-0.5" />
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
        on:click={() => dispatch("note")}
        disabled={!connected || hasWriteAccess === false}
        title="Add sticky note"
      >
        <FileTextIcon strokeWidth={1.5} class="p-0.5" />
      </button>
      <button
        class="icon-button"
        on:click={() => dispatch("video")}
        disabled={!connected || hasWriteAccess === false}
        title="Add video to board"
      >
        <FilmIcon strokeWidth={1.5} class="p-0.5" />
      </button>
      <button
        class="icon-button"
        class:active={cameraActive}
        on:click={() => dispatch("camera")}
        disabled={!connected}
        title={cameraActive ? "Stop camera" : "Start camera"}
      >
        <VideoIcon strokeWidth={1.5} class="p-0.5" />
      </button>
    </div>

    <div class="v-divider" />

    <div class="flex space-x-1">
      <button class="icon-button" on:click={() => dispatch("networkInfo")}>
        <WifiIcon strokeWidth={1.5} class="p-0.5" />
      </button>
    </div>
    {/if}
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

  .icon-button.lock-on {
    @apply bg-amber-500 text-zinc-900 hover:bg-amber-400;
  }

  .activity {
    @apply absolute top-0.5 right-0.5 p-[4px] bg-red-500 rounded-full;
  }
</style>
