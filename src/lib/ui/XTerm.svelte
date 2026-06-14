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
    snap: string;
    showSnapPad: void;
    hideSnapPad: void;
  }>();

  // Quick shape presets — a tall portrait rectangle and a wide landscape one
  // (Bo 2026-06-13: "สี่เหลี่ยมผืนผ้าแนวตั้งกับแนวนอน 2 ไซส์").
  const SIZE_PRESETS = [
    { label: "▯", title: "Portrait — tall (72×48)", cols: 72, rows: 48 },
    { label: "▭", title: "Landscape — wide (150×28)", cols: 150, rows: 28 },
  ];

  // Rectangle-style snap layouts — tap to slot this terminal into a region of
  // the visible board (Bo 2026-06-13). The parent (Session.svelte) maps the
  // action onto the current viewport + resizes the terminal to fit. Grid mirrors
  // the Rectangle menu: halves, quarters, thirds, and restore.
  const SNAP_GROUPS = [
    [
      { a: "leftHalf", g: "◧", t: "Left half" },
      { a: "rightHalf", g: "◨", t: "Right half" },
      { a: "topHalf", g: "⬒", t: "Top half" },
      { a: "bottomHalf", g: "⬓", t: "Bottom half" },
      { a: "topLeft", g: "◰", t: "Top-left quarter" },
      { a: "topRight", g: "◳", t: "Top-right quarter" },
      { a: "bottomLeft", g: "◱", t: "Bottom-left quarter" },
      { a: "bottomRight", g: "◲", t: "Bottom-right quarter" },
    ],
    [
      { a: "firstThird", g: "⅓", t: "First third (repeat cycles)" },
      { a: "centerThird", g: "⅓", t: "Center third" },
      { a: "lastThird", g: "⅓", t: "Last third (repeat cycles)" },
      { a: "firstTwoThirds", g: "⅔", t: "First two-thirds" },
      { a: "centerTwoThirds", g: "⅔", t: "Center two-thirds" },
      { a: "lastTwoThirds", g: "⅔", t: "Last two-thirds" },
    ],
    [
      { a: "maximize", g: "⬜", t: "Maximize" },
      { a: "almostMaximize", g: "▣", t: "Almost maximize" },
      { a: "maximizeHeight", g: "⇕", t: "Maximize height" },
      { a: "center", g: "⊡", t: "Center" },
      { a: "restore", g: "↩", t: "Restore previous layout" },
    ],
  ];

  const SNAP_PAD_GROUPS = [
    [
      { a: "leftHalf", g: "◧", k: "⌃⌥←", t: "Left half" },
      { a: "rightHalf", g: "◨", k: "⌃⌥→", t: "Right half" },
      { a: "topHalf", g: "⬒", k: "⌃⌥↑", t: "Top half" },
      { a: "bottomHalf", g: "⬓", k: "⌃⌥↓", t: "Bottom half" },
    ],
    [
      { a: "topLeft", g: "◰", k: "⌃⌥U", t: "Top-left quarter" },
      { a: "topRight", g: "◳", k: "⌃⌥I", t: "Top-right quarter" },
      { a: "bottomLeft", g: "◱", k: "⌃⌥J", t: "Bottom-left quarter" },
      { a: "bottomRight", g: "◲", k: "⌃⌥K", t: "Bottom-right quarter" },
    ],
    [
      { a: "maximize", g: "⬜", k: "⌃⌥F", t: "Maximize" },
      { a: "center", g: "⊡", k: "⌃⌥C", t: "Center" },
      { a: "firstThird", g: "⅓", k: "⌃⌥1", t: "First third" },
      { a: "centerThird", g: "⅓", k: "⌃⌥2", t: "Center third" },
      { a: "lastThird", g: "⅓", k: "⌃⌥3", t: "Last third" },
      { a: "restore", g: "↩", k: "⌃⌥0", t: "Restore previous layout" },
    ],
  ];
  let snapOpen = false;

  const typeahead = new TypeAheadAddon();

  export let rows: number, cols: number;
  export let write: (data: string) => void; // bound function prop
  export let label = ""; // user-set terminal name, synced across peers
  export let canRename = true;
  export let snapPadOpen = false;

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
      dispatch("showSnapPad");
    } else if (!isFocused && focused) {
      focused = isFocused;
      cursorLayer.addEventListener("wheel", handleWheelSkipXTerm);
      dispatch("blur");
      dispatch("hideSnapPad");
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
      if (event.key === "Escape" && snapPadOpen) {
        dispatch("hideSnapPad");
      } else if (
        snapPadOpen &&
        !event.ctrlKey &&
        !event.altKey &&
        !event.metaKey &&
        (event.key.length === 1 ||
          event.key === "Enter" ||
          event.key === "Backspace")
      ) {
        dispatch("hideSnapPad");
      }
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
    term.loadAddon(new ImageAddon({ enableSizeReports: false }));

    term.open(termEl);

    // WebGL renderer AFTER open(), wrapped — iOS/iPadOS Safari caps live WebGL
    // contexts (~8–16/page), so on a board with many terminals the overflow
    // ones fail to get a context. If that throw happens during open() the whole
    // terminal breaks (blank/white or header-only); loading WebGL separately and
    // catching lets those terminals fall back to xterm's DOM renderer and still
    // render. onContextLoss handles a context dropped later at runtime.
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      term.loadAddon(webgl);
    } catch {
      // No WebGL available — xterm keeps its default (DOM) renderer.
    }

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
  on:pointerdown={(event) => {
    dispatch("showSnapPad");
    event.stopPropagation();
  }}
>
  <div
    class="flex select-none touch-none"
    on:pointerdown={(event) => {
      // Ignore real controls (window buttons, size presets, rename input) so
      // they fire their own action. The title is the biggest part of the
      // header, so it must stay draggable — a tap without movement still
      // renames. Hence button:not(.term-title).
      if (
        event.target instanceof HTMLElement &&
        event.target.closest("button:not(.term-title), input")
      )
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
          class="term-title w-full truncate {label ? 'text-indigo-300' : 'text-zinc-300'} hover:text-white"
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
          title={p.title}
          on:pointerdown={(event) => {
            if (event.button !== 0) return;
            event.stopPropagation();
            dispatch("preset", { cols: p.cols, rows: p.rows });
          }}
        >
          {p.label}
        </button>
      {/each}
      <!-- Rectangle-style snap menu — tap ⊞ to slot this terminal into a region. -->
      <div class="relative">
        <button
          class="size-preset"
          class:active={snapOpen}
          title="Snap layout"
          on:pointerdown={(event) => {
            if (event.button !== 0) return;
            event.stopPropagation();
            snapOpen = !snapOpen;
          }}
        >
          ⊞
        </button>
        {#if snapOpen}
          <div
            class="absolute top-full right-0 mt-1 z-50 flex flex-col gap-1 p-1.5 rounded-lg bg-zinc-800 border border-zinc-700 shadow-xl"
            on:pointerdown={(e) => e.stopPropagation()}
          >
            {#each SNAP_GROUPS as group}
              <div class="grid grid-cols-4 gap-1">
                {#each group as s}
                  <button
                    class="w-7 h-7 flex items-center justify-center rounded text-zinc-300 hover:bg-indigo-600 hover:text-white text-sm leading-none"
                    title={s.t}
                    on:pointerdown={(event) => {
                      if (event.button !== 0) return;
                      event.stopPropagation();
                      dispatch("snap", s.a);
                      snapOpen = false;
                    }}
                  >
                    {s.g}
                  </button>
                {/each}
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  </div>
  {#if snapPadOpen}
    <div
      class="snap-pad"
      on:pointerdown={(event) => event.stopPropagation()}
      on:mousedown={(event) => event.stopPropagation()}
    >
      <div class="snap-pad-head">
        <span>Snap</span>
        <button
          class="snap-pad-close"
          title="Hide snap pad"
          on:pointerdown={(event) => {
            if (event.button !== 0) return;
            event.stopPropagation();
            dispatch("hideSnapPad");
          }}
        >
          ×
        </button>
      </div>
      {#each SNAP_PAD_GROUPS as group}
        <div class="snap-pad-grid">
          {#each group as s}
            <button
              class="snap-pad-btn"
              title={s.t}
              on:pointerdown={(event) => {
                if (event.button !== 0) return;
                event.preventDefault();
                event.stopPropagation();
                dispatch("snap", s.a);
                dispatch("hideSnapPad");
                snapOpen = false;
              }}
            >
              <span class="snap-pad-icon">{s.g}</span>
              <span class="snap-pad-key">{s.k}</span>
            </button>
          {/each}
        </div>
      {/each}
    </div>
  {/if}
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
    @apply relative inline-block rounded-lg border border-zinc-700 opacity-90;
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

  .snap-pad {
    @apply absolute z-50 top-10 left-2 right-2 max-w-[25rem] p-2 rounded-lg;
    @apply border border-zinc-700 bg-zinc-900/95 shadow-2xl backdrop-blur-md;
    @apply flex flex-col gap-1.5;
  }

  .snap-pad-head {
    @apply flex items-center justify-between px-1 pb-0.5 text-[11px] font-semibold text-zinc-400;
  }

  .snap-pad-close {
    @apply w-7 h-7 rounded-md text-zinc-400 hover:bg-zinc-800 hover:text-zinc-100;
  }

  .snap-pad-grid {
    @apply grid grid-cols-3 sm:grid-cols-4 gap-1.5;
  }

  .snap-pad-btn {
    @apply h-12 min-w-[4.25rem] rounded-md border border-zinc-700/70;
    @apply bg-zinc-800/80 text-zinc-100 hover:bg-indigo-600 hover:border-indigo-500;
    @apply flex flex-col items-center justify-center gap-0.5 touch-manipulation;
  }

  .snap-pad-icon {
    @apply text-base leading-none;
  }

  .snap-pad-key {
    @apply text-[10px] leading-none font-semibold text-zinc-400;
  }

  .snap-pad-btn:hover .snap-pad-key {
    @apply text-indigo-100;
  }

  @media (hover: none), (pointer: coarse) {
    .snap-pad {
      @apply top-11 p-2.5 gap-2;
    }

    .snap-pad-grid {
      @apply gap-2;
    }

    .snap-pad-btn {
      @apply h-14 min-w-[4.8rem];
    }
  }
</style>
