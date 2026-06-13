<!-- @component Interactive terminal rendered with xterm.js -->
<script lang="ts" context="module">
  import { makeToast } from "$lib/toast";

  // Deduplicated terminal font loading.
  const waitForFonts = (() => {
    let state: "initial" | "loading" | "loaded" = "initial";
    const waitlist: (() => void)[] = [];

    return async function waitForFonts() {
      if (state === "loaded") return;
      else if (state === "initial") {
        const FontFaceObserver = (await import("fontfaceobserver")).default;
        state = "loading";
        try {
          await new FontFaceObserver("Fira Code VF").load();
        } catch (error) {
          makeToast({
            kind: "error",
            message: "Could not load terminal font.",
          });
        }
        state = "loaded";
        for (const fn of waitlist) fn();
      } else {
        await new Promise<void>((resolve) => {
          if (state === "loaded") resolve();
          else waitlist.push(resolve);
        });
      }
    };
  })();
</script>

<script lang="ts">
  import { browser } from "$app/environment";

  import { createEventDispatcher, onDestroy, onMount } from "svelte";
  import type { Terminal } from "sshx-xterm";
  import { Buffer } from "buffer";

  import themes from "./themes";
  import CircleButton from "./CircleButton.svelte";
  import CircleButtons from "./CircleButtons.svelte";
  import { settings } from "$lib/settings";
  import { TypeAheadAddon } from "$lib/typeahead";

  /** Used to determine Cmd versus Ctrl keyboard shortcuts. */
  const isMac = browser && navigator.platform.startsWith("Mac");

  const dispatch = createEventDispatcher<{
    data: Uint8Array;
    close: void;
    shrink: void;
    expand: void;
    bringToFront: void;
    startMove: MouseEvent;
    preset: { cols: number; rows: number };
    focus: void;
    blur: void;
    rename: string;
  }>();

  // Quick terminal size presets (cols × rows).
  const SIZE_PRESETS = [
    { label: "S", cols: 80, rows: 24 },
    { label: "M", cols: 100, rows: 30 },
    { label: "L", cols: 120, rows: 40 },
    { label: "XL", cols: 160, rows: 48 },
  ];

  const typeahead = new TypeAheadAddon();

  export let rows: number, cols: number;
  export let write: (data: string) => void; // bound function prop
  export let label = ""; // user-set terminal name, synced across peers
  export let canRename = true;

  let renaming = false;
  let draftLabel = "";
  function startRename() {
    if (!canRename) return;
    draftLabel = label;
    renaming = true;
  }
  function commitRename() {
    if (!renaming) return;
    renaming = false;
    dispatch("rename", draftLabel.trim());
  }

  export let termEl: HTMLDivElement = null as any; // suppress "missing prop" warning
  let term: Terminal | null = null;

  $: theme = themes[$settings.theme];

  $: if (term) {
    // If the theme changes, update existing terminals' appearance.
    term.options.theme = theme;
    term.options.scrollback = $settings.scrollback;
    term.options.fontSize = $settings.fontSize;
  }

  let loaded = false;
  let focused = false;
  let currentTitle = "Remote Terminal";

  function handleWheelSkipXTerm(event: WheelEvent) {
    event.preventDefault(); // Stop native macOS Chrome zooming on pinch.

    // We stop the event from propagating to the main `.xterm` terminal element,
    // so the xterm.js's event handlers do not fire and scroll the buffer.
    event.stopPropagation();

    // However, we still want it to propagate upward to our pan/zoom handlers,
    // so we re-dispatch the event higher up, skipping xterm.
    termEl?.dispatchEvent(new WheelEvent(event.type, event));
  }

  function setFocused(isFocused: boolean, cursorLayer: HTMLDivElement) {
    if (isFocused && !focused) {
      focused = isFocused;
      cursorLayer.removeEventListener("wheel", handleWheelSkipXTerm);
      dispatch("focus");
    } else if (!isFocused && focused) {
      focused = isFocused;
      cursorLayer.addEventListener("wheel", handleWheelSkipXTerm);
      dispatch("blur");
    }
  }

  const preloadBuffer: string[] = [];

  write = (data: string) => {
    if (!term) {
      // Before the terminal is loaded, push data into a buffer.
      preloadBuffer.push(data);
    } else {
      if (data) data = typeahead.onBeforeProcessData(data);
      term.write(data);
    }
  };

  $: term?.resize(cols, rows);

  onMount(async () => {
    const [{ Terminal }, { WebLinksAddon }, { WebglAddon }, { ImageAddon }] =
      await Promise.all([
        import("sshx-xterm"),
        import("xterm-addon-web-links"),
        import("xterm-addon-webgl"),
        import("xterm-addon-image"),
      ]);

    await waitForFonts();

    term = new Terminal({
      allowTransparency: false,
      cursorBlink: false,
      cursorStyle: "block",
      // This is the monospace font family configured in Tailwind.
      fontFamily:
        '"Fira Code VF", ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace',
      fontSize: $settings.fontSize,
      fontWeight: 400,
      fontWeightBold: 500,
      lineHeight: 1.06,
      scrollback: $settings.scrollback,
      theme,
    });

    // Keyboard shortcuts for natural text editing.
    term.attachCustomKeyEventHandler((event) => {
      if (
        (isMac && event.metaKey && !event.ctrlKey && !event.altKey) ||
        (!isMac && !event.metaKey && event.ctrlKey && !event.altKey)
      ) {
        if (event.key === "ArrowLeft") {
          dispatch("data", new Uint8Array([0x01]));
          return false;
        } else if (event.key === "ArrowRight") {
          dispatch("data", new Uint8Array([0x05]));
          return false;
        } else if (event.key === "Backspace") {
          dispatch("data", new Uint8Array([0x15]));
          return false;
        }
      }
      return true;
    });

    term.loadAddon(new WebLinksAddon());
    term.loadAddon(new WebglAddon());
    term.loadAddon(new ImageAddon({ enableSizeReports: false }));

    term.open(termEl);

    term.resize(cols, rows);
    term.onTitleChange((title) => {
      currentTitle = title;
    });

    // Hack: We artificially disable scrolling when the terminal is not focused.
    // ("termEl" > div.terminal.xterm > div.xterm-screen)
    const screenEl = termEl.querySelector(".xterm-screen")! as HTMLDivElement;
    screenEl.addEventListener("wheel", handleWheelSkipXTerm);

    const focusObserver = new MutationObserver((mutations) => {
      for (const mutation of mutations) {
        if (
          mutation.type === "attributes" &&
          mutation.attributeName === "class"
        ) {
          // The "focus" class is set directly by xterm.js, but there isn't any way to listen for it.
          const target = mutation.target as HTMLElement;
          const isFocused = target.classList.contains("focus");
          setFocused(isFocused, screenEl);
        }
      }
    });
    focusObserver.observe(term.element!, { attributeFilter: ["class"] });

    loaded = true;
    for (const data of preloadBuffer) {
      term.write(data);
    }

    typeahead.reset();
    term.loadAddon(typeahead);

    const utf8 = new TextEncoder();
    term.onData((data: string) => {
      dispatch("data", utf8.encode(data));
    });
    term.onBinary((data: string) => {
      dispatch("data", Buffer.from(data, "binary"));
    });
  });

  onDestroy(() => term?.dispose());
</script>

<div
  class="term-container"
  class:focused
  style:background={theme.background}
  on:mousedown={() => dispatch("bringToFront")}
  on:pointerdown={(event) => event.stopPropagation()}
>
  <div
    class="flex select-none touch-none"
    on:pointerdown={(event) => {
      // Ignore taps on the control buttons (close/shrink/expand/presets) so
      // they fire their own action instead of starting a window drag.
      if ((event.target instanceof HTMLElement) && event.target.closest("button"))
        return;
      dispatch("startMove", event);
    }}
  >
    <div class="flex-1 flex items-center px-3">
      <CircleButtons>
        <!--
          TODO: This should be on:click, but that is not working due to the
          containing element's on:pointerdown `stopPropagation()` call.
        -->
        <CircleButton
          kind="red"
          on:mousedown={(event) => event.button === 0 && dispatch("close")}
        />
        <CircleButton
          kind="yellow"
          on:mousedown={(event) => event.button === 0 && dispatch("shrink")}
        />
        <CircleButton
          kind="green"
          on:mousedown={(event) => event.button === 0 && dispatch("expand")}
        />
      </CircleButtons>
    </div>
    <div
      class="p-2 text-sm text-center font-medium overflow-hidden whitespace-nowrap text-ellipsis w-0 flex-grow-[4]"
    >
      {#if renaming}
        <!-- svelte-ignore a11y-autofocus -->
        <input
          class="w-full bg-zinc-900 text-zinc-100 text-center rounded px-1 outline-none ring-1 ring-indigo-500"
          bind:value={draftLabel}
          autofocus
          spellcheck="false"
          placeholder="name this terminal…"
          on:pointerdown={(e) => e.stopPropagation()}
          on:blur={commitRename}
          on:keydown={(e) => {
            if (e.key === "Enter") commitRename();
            else if (e.key === "Escape") renaming = false;
          }}
        />
      {:else}
        <button
          class="w-full truncate {label ? 'text-indigo-300' : 'text-zinc-300'} hover:text-white"
          title={canRename ? "Click to rename" : currentTitle}
          on:click={startRename}
        >
          {label || currentTitle}
        </button>
      {/if}
    </div>
    <div class="flex-1 flex items-center justify-end pr-3 gap-1">
      {#each SIZE_PRESETS as p}
        <button
          class="size-preset"
          class:active={cols === p.cols && rows === p.rows}
          title={`Resize terminal to ${p.cols}×${p.rows}`}
          on:pointerdown={(event) => {
            if (event.button !== 0) return;
            event.stopPropagation();
            dispatch("preset", { cols: p.cols, rows: p.rows });
          }}
        >
          {p.label}
        </button>
      {/each}
    </div>
  </div>
  <div
    class="inline-block px-4 py-2 transition-opacity duration-500"
    bind:this={termEl}
    style:opacity={loaded ? 1.0 : 0.0}
    on:wheel={(event) => {
      if (focused) {
        // Don't pan the page when scrolling while the terminal is selected.
        // Conversely, we manually disable terminal scrolling unless it is currently selected.
        event.stopPropagation();
      }
    }}
  />
</div>

<style lang="postcss">
  .term-container {
    @apply inline-block rounded-lg border border-zinc-700 opacity-90;
    transition: transform 200ms, opacity 200ms;
  }

  .size-preset {
    @apply rounded-md px-1.5 py-0.5 text-xs font-medium text-zinc-400;
    @apply hover:bg-zinc-700/60 hover:text-zinc-100 transition-colors;
  }

  .size-preset.active {
    @apply bg-indigo-600/80 text-white;
  }

  .term-container:not(.focused) :global(.xterm) {
    @apply cursor-default;
  }

  .term-container.focused {
    @apply opacity-100;
  }
</style>
