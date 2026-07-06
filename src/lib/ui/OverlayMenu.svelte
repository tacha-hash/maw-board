<script lang="ts">
  // Hand-rolled replacement for @rgossiaux/svelte-headlessui's Dialog +
  // Transition suite (library is dead on Svelte 5 — imports svelte/internal).
  // Same behavior: backdrop blur overlay, click-outside + Escape to close,
  // scale/fade enter-leave.
  import { XIcon } from "svelte-feather-icons";
  import { createEventDispatcher } from "svelte";
  import { fade, scale } from "svelte/transition";

  const dispatch = createEventDispatcher<{ close: void }>();

  export let title: string;
  export let description: string;
  export let showCloseButton = false;
  export let maxWidth: number = 768; // screen-md
  export let open: boolean;

  function onKeydown(event: KeyboardEvent) {
    if (open && event.key === "Escape") dispatch("close");
  }
</script>

<svelte:window on:keydown={onKeydown} />

{#if open}
  <div class="fixed inset-0 z-50 grid place-items-center" role="dialog" aria-modal="true">
    <div
      class="fixed -z-10 inset-0 bg-black/20 backdrop-blur-sm"
      transition:fade={{ duration: 150 }}
      on:click={() => dispatch("close")}
      aria-hidden="true"
    />

    <div
      class="w-full sm:w-[calc(100%-32px)]"
      style="max-width: {maxWidth}px"
      transition:scale={{ start: 0.95, duration: 200 }}
    >
      <div
        class="relative bg-[#111] sm:border border-zinc-800 px-6 py-10 sm:py-6
         h-screen sm:h-auto max-h-screen sm:rounded-lg overflow-y-auto"
      >
        {#if showCloseButton}
          <button
            class="absolute top-4 right-4 p-1 rounded hover:bg-zinc-700 active:bg-indigo-700 transition-colors"
            aria-label="Close {title}"
            on:click={() => dispatch("close")}
          >
            <XIcon class="h-5 w-5" />
          </button>
        {/if}

        <div class="mb-8 text-center">
          <h2 class="text-xl font-medium mb-2">
            {title}
          </h2>
          <p class="text-zinc-400">
            {description}
          </p>
        </div>

        <slot />
      </div>
    </div>
  </div>
{/if}
