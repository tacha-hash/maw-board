<script lang="ts">
  import { ChevronDownIcon } from "svelte-feather-icons";

  import { settings, updateSettings } from "$lib/settings";
  import OverlayMenu from "./OverlayMenu.svelte";
  import themes, { type ThemeName } from "./themes";

  export let open: boolean;

  let inputName: string;
  let inputTheme: ThemeName;
  let inputScrollback: number;
  let inputFontSize: number;
  let inputBackground: string;
  let inputPanelBackground: string;

  // Quick board background presets.
  const BG_PRESETS = [
    { label: "Charcoal", value: "#0e0e10" },
    { label: "Midnight", value: "#0b1220" },
    { label: "Forest", value: "#0c1410" },
    { label: "Plum", value: "#140d18" },
    { label: "Slate", value: "#1a1d23" },
    { label: "Black", value: "#000000" },
  ];

  function setBackground(value: string) {
    inputBackground = value;
    updateSettings({ background: value });
  }

  function setPanelBackground(value: string) {
    inputPanelBackground = value;
    updateSettings({ panelBackground: value });
  }

  let initialized = false;
  $: open, (initialized = false);
  $: if (!initialized) {
    initialized = true;
    inputName = $settings.name;
    inputTheme = $settings.theme;
    inputScrollback = $settings.scrollback;
    inputFontSize = $settings.fontSize;
    inputBackground = $settings.background;
    inputPanelBackground = $settings.panelBackground;
  }
</script>

<OverlayMenu
  title="Terminal Settings"
  description="Customize your collaborative terminal."
  showCloseButton
  {open}
  on:close
>
  <div class="flex flex-col gap-4">
    <div class="item">
      <div>
        <p class="item-title">Name</p>
        <p class="item-subtitle">Choose how you appear to other users.</p>
      </div>
      <div>
        <input
          class="input-common"
          placeholder="Your name"
          bind:value={inputName}
          maxlength="50"
          on:input={() => {
            if (inputName.length >= 2) {
              updateSettings({ name: inputName });
            }
          }}
        />
      </div>
    </div>
    <div class="item">
      <div>
        <p class="item-title">Color palette</p>
        <p class="item-subtitle">Color theme for text in terminals.</p>
      </div>
      <div class="relative">
        <ChevronDownIcon
          class="absolute top-[11px] right-2.5 w-4 h-4 text-zinc-400"
        />
        <select
          class="input-common !pr-5"
          bind:value={inputTheme}
          on:change={() => updateSettings({ theme: inputTheme })}
        >
          {#each Object.keys(themes) as themeName (themeName)}
            <option value={themeName}>{themeName}</option>
          {/each}
        </select>
      </div>
    </div>
    <div class="item">
      <div>
        <p class="item-title">Font size</p>
        <p class="item-subtitle">Terminal text size ({inputFontSize}px).</p>
      </div>
      <div class="flex items-center gap-3 w-52">
        <input
          type="range"
          min="8"
          max="40"
          step="1"
          class="flex-1 accent-indigo-500"
          bind:value={inputFontSize}
          on:input={() => updateSettings({ fontSize: Number(inputFontSize) })}
        />
        <span class="text-sm text-zinc-300 w-8 text-right">{inputFontSize}</span>
      </div>
    </div>
    <div class="item">
      <div>
        <p class="item-title">Board background</p>
        <p class="item-subtitle">Background color of the board canvas.</p>
      </div>
      <div class="flex flex-col gap-2 items-start">
        <div class="flex gap-1.5 flex-wrap w-52">
          {#each BG_PRESETS as preset}
            <button
              class="bg-swatch"
              class:active={inputBackground === preset.value}
              style:background-color={preset.value}
              title={preset.label}
              on:click={() => setBackground(preset.value)}
            />
          {/each}
        </div>
        <input
          type="color"
          class="color-input"
          bind:value={inputBackground}
          on:input={() => setBackground(inputBackground)}
        />
      </div>
    </div>
    <div class="item">
      <div>
        <p class="item-title">Panel color</p>
        <p class="item-subtitle">Toolbar, window headers and menu panels.</p>
      </div>
      <div class="flex items-center gap-2 w-52">
        <input
          type="color"
          class="color-input flex-1"
          value={inputPanelBackground || "#18181b"}
          on:input={(e) => setPanelBackground(e.currentTarget.value)}
        />
        <button
          class="reset-btn"
          title="Reset to default"
          on:click={() => setPanelBackground("")}
        >
          Default
        </button>
      </div>
    </div>
    <div class="item">
      <div>
        <p class="item-title">Scrollback</p>
        <p class="item-subtitle">
          Lines of previous text displayed in the terminal window.
        </p>
      </div>
      <div>
        <input
          type="number"
          class="input-common"
          bind:value={inputScrollback}
          on:input={() => {
            if (inputScrollback >= 0) {
              updateSettings({ scrollback: inputScrollback });
            }
          }}
          step="100"
        />
      </div>
    </div>
    <!-- <div class="item">
      <div>
        <p class="item-title">Cursor style</p>
        <p class="item-subtitle">Style of live cursors.</p>
      </div>
      <div class="text-red-500">Coming soon</div>
    </div> -->
  </div>

  <!-- svelte-ignore missing-declaration -->
  <p class="mt-6 text-sm text-right text-zinc-400">
    Oracle Board v{__APP_VERSION__}
  </p>
</OverlayMenu>

<style lang="postcss">
  .item {
    @apply bg-zinc-800/25 rounded-lg p-4 flex gap-4 flex-col sm:flex-row items-start;
  }

  .item > div:first-child {
    @apply flex-1;
  }

  .item-title {
    @apply font-medium text-zinc-200 mb-1;
  }

  .item-subtitle {
    @apply text-sm text-zinc-400;
  }

  .input-common {
    @apply w-52 px-3 py-2 text-sm rounded-md bg-transparent hover:bg-white/5;
    @apply border border-zinc-700 outline-none focus:ring-2 focus:ring-indigo-500/50;
    @apply appearance-none transition-colors;
  }

  .bg-swatch {
    @apply w-7 h-7 rounded-md border border-zinc-600 transition-transform;
    @apply hover:scale-110;
  }

  .bg-swatch.active {
    @apply ring-2 ring-indigo-500 ring-offset-1 ring-offset-zinc-900;
  }

  .color-input {
    @apply w-52 h-9 rounded-md border border-zinc-700 bg-transparent cursor-pointer p-1;
  }

  .reset-btn {
    @apply px-2.5 py-1.5 text-xs rounded-md border border-zinc-700 text-zinc-300;
    @apply hover:bg-white/5 transition-colors whitespace-nowrap;
  }
</style>
