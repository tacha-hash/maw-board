<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import StatusBar from "./StatusBar.svelte";
  import LayoutMenu from "./LayoutMenu.svelte";
  import OracleBoardMenu from "./OracleBoardMenu.svelte";
  import {
    ChevronDownIcon,
    ChevronLeftIcon,
    ChevronRightIcon,
    CrosshairIcon,
    Edit2Icon,
    FileTextIcon,
    FilmIcon,
    FolderIcon,
    ClipboardIcon,
    GridIcon,
    LockIcon,
    UnlockIcon,
    Trash2Icon,
    ImageIcon,
    MessageSquareIcon,
    MicIcon,
    PlusCircleIcon,
    RadioIcon,
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
  export let broadcastMode = false;

  const dispatch = createEventDispatcher<{
    create: void;
    lock: void;
    broadcast: void;
    snippets: void;
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
  let brandMenuOpen = false;
</script>

<!-- Single row + horizontal scroll on small screens. While a dropdown is open
     we switch overflow to visible so the menu isn't clipped (you don't scroll
     the toolbar while a menu is open anyway). -->
<div
  class="panel inline-block px-3 py-2 max-w-[calc(100vw-12px)] toolbar-scroll"
  style="overflow-x: {brandMenuOpen || showTileMenu ? 'visible' : 'auto'};"
>
  <div class="flex flex-nowrap items-center select-none">
    <div class="brand-wrap">
      <button
        class="brand"
        class:open={brandMenuOpen}
        on:click={() => (brandMenuOpen = !brandMenuOpen)}
        title="Oracle Board — system monitor"
      >
        <TerminalIcon size="18" strokeWidth={2} />
        <span class="hidden sm:inline">Oracle Board</span>
        <ChevronDownIcon size="14" strokeWidth={2} class="chev" />
      </button>
      <OracleBoardMenu open={brandMenuOpen} />
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

      <div class="flex gap-1">
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
        disabled={!connected || hasWriteAccess === false}
        title={boardLocked
          ? "Board locked — click to unlock"
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
        class:broadcast-on={broadcastMode}
        on:click={() => dispatch("broadcast")}
        disabled={!connected || hasWriteAccess === false}
        title={broadcastMode
          ? "Broadcast ON — typing goes to ALL terminals (click to stop)"
          : "Broadcast input to all terminals at once"}
      >
        <RadioIcon strokeWidth={1.5} class="p-0.5" />
      </button>
      <button
        class="icon-button"
        on:click={() => dispatch("snippets")}
        title="Command snippets — click to paste"
      >
        <ClipboardIcon strokeWidth={1.5} class="p-0.5" />
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
  .brand-wrap {
    @apply relative;
  }
  .brand {
    @apply flex items-center gap-1.5 text-indigo-400 font-semibold text-sm tracking-tight;
    @apply rounded-lg px-1.5 py-1 hover:bg-zinc-800/70 transition-colors;
  }
  .brand.open {
    @apply bg-zinc-800 text-indigo-300;
  }
  .brand :global(.chev) {
    @apply text-zinc-500 transition-transform duration-200;
  }
  .brand.open :global(.chev) {
    @apply rotate-180 text-indigo-300;
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

  .icon-button.broadcast-on {
    @apply bg-red-600 text-white hover:bg-red-500;
  }

  .activity {
    @apply absolute top-0.5 right-0.5 p-[4px] bg-red-500 rounded-full;
  }

  .toolbar-scroll {
    touch-action: pan-x;
    scrollbar-width: thin;
  }

  /* Touch / coarse pointer: ~44px hit areas without growing the icon glyphs. */
  @media (hover: none), (pointer: coarse) {
    .icon-button {
      @apply min-w-[44px] min-h-[44px] flex items-center justify-center;
    }
  }
</style>
