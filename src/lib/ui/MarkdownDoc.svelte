<!--
  Shared markdown document panel — a collaborative scratch doc with a live
  preview. The text is synced to all peers (stored as a singleton board item).
  Markdown is rendered by a small, dependency-free converter that escapes HTML
  first, so peer-authored content can't inject markup.
-->
<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { XIcon, EyeIcon, Edit2Icon } from "svelte-feather-icons";

  export let text: string = "";
  export let readonly: boolean = false;

  const dispatch = createEventDispatcher<{ close: void; edit: string }>();

  let mode: "split" | "edit" | "preview" = "split";

  function escapeHtml(s: string): string {
    return s
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }

  // Inline: code, bold, italic, links (http/https only).
  function inline(s: string): string {
    return s
      .replace(/`([^`]+)`/g, "<code>$1</code>")
      .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
      .replace(/\*([^*]+)\*/g, "<em>$1</em>")
      .replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, (_m, t, u) => {
        return `<a href="${u}" target="_blank" rel="noopener noreferrer">${t}</a>`;
      });
  }

  // Minimal block-level markdown → HTML. Input is escaped up front.
  function mdToHtml(src: string): string {
    const lines = escapeHtml(src).split("\n");
    const out: string[] = [];
    let inCode = false;
    let inList = false;
    const closeList = () => {
      if (inList) {
        out.push("</ul>");
        inList = false;
      }
    };
    for (const line of lines) {
      if (line.trim().startsWith("```")) {
        if (inCode) {
          out.push("</code></pre>");
          inCode = false;
        } else {
          closeList();
          out.push("<pre><code>");
          inCode = true;
        }
        continue;
      }
      if (inCode) {
        out.push(line + "\n");
        continue;
      }
      const h = line.match(/^(#{1,4})\s+(.*)$/);
      if (h) {
        closeList();
        const lvl = h[1].length;
        out.push(`<h${lvl}>${inline(h[2])}</h${lvl}>`);
        continue;
      }
      const li = line.match(/^\s*[-*]\s+(.*)$/);
      if (li) {
        if (!inList) {
          out.push("<ul>");
          inList = true;
        }
        out.push(`<li>${inline(li[1])}</li>`);
        continue;
      }
      closeList();
      if (line.trim() === "") out.push("<br/>");
      else out.push(`<p>${inline(line)}</p>`);
    }
    if (inCode) out.push("</code></pre>");
    closeList();
    return out.join("");
  }

  $: html = mdToHtml(text);
</script>

<div class="doc panel">
  <div class="head">
    <div class="flex items-center gap-1.5 text-zinc-200 font-medium text-sm">
      <Edit2Icon size="14" />
      <span>Document</span>
    </div>
    <div class="flex items-center gap-1">
      <button
        class="mode"
        class:on={mode === "edit"}
        title="Edit only"
        on:click={() => (mode = "edit")}><Edit2Icon size="13" /></button>
      <button
        class="mode"
        class:on={mode === "split"}
        title="Split"
        on:click={() => (mode = "split")}>◧</button>
      <button
        class="mode"
        class:on={mode === "preview"}
        title="Preview only"
        on:click={() => (mode = "preview")}><EyeIcon size="13" /></button>
      <button class="close" title="Close" on:click={() => dispatch("close")}>
        <XIcon size="15" />
      </button>
    </div>
  </div>

  <div class="body" class:split={mode === "split"}>
    {#if mode !== "preview"}
      <textarea
        class="src"
        placeholder="# Notes&#10;Write **markdown** here…"
        value={text}
        {readonly}
        on:input={(e) => dispatch("edit", e.currentTarget.value)}
      />
    {/if}
    {#if mode !== "edit"}
      <!-- eslint-disable-next-line svelte/no-at-html-tags -->
      <div class="preview">{@html html}</div>
    {/if}
  </div>
</div>

<style lang="postcss">
  .doc {
    @apply fixed left-2 right-2 sm:left-auto sm:right-4 top-20 bottom-4 w-auto sm:w-[28rem] z-40 flex flex-col p-0 overflow-hidden;
  }
  .head {
    @apply flex items-center justify-between px-3 py-2 border-b border-zinc-700/60;
  }
  .mode {
    @apply w-6 h-6 grid place-items-center rounded text-xs text-zinc-400 hover:bg-white/5;
  }
  .mode.on {
    @apply bg-indigo-600/70 text-white;
  }
  .close {
    @apply rounded-md p-0.5 text-zinc-400 hover:text-white hover:bg-zinc-700/60 ml-1;
  }
  .body {
    @apply flex-1 min-h-0 flex;
  }
  .body.split .src {
    @apply w-1/2 border-r border-zinc-700/60;
  }
  .src {
    @apply flex-1 p-3 bg-transparent resize-none outline-none text-sm font-mono;
    @apply text-zinc-200 leading-relaxed;
  }
  .preview {
    @apply flex-1 p-3 overflow-y-auto text-sm text-zinc-300 leading-relaxed;
  }
  .preview :global(h1) {
    @apply text-lg font-semibold text-zinc-100 mt-2 mb-1;
  }
  .preview :global(h2) {
    @apply text-base font-semibold text-zinc-100 mt-2 mb-1;
  }
  .preview :global(h3),
  .preview :global(h4) {
    @apply font-semibold text-zinc-200 mt-1.5;
  }
  .preview :global(p) {
    @apply my-1;
  }
  .preview :global(ul) {
    @apply list-disc pl-5 my-1;
  }
  .preview :global(code) {
    @apply bg-zinc-800 rounded px-1 py-0.5 text-[0.85em] text-amber-300 font-mono;
  }
  .preview :global(pre) {
    @apply bg-zinc-800/80 rounded-md p-2 my-1.5 overflow-x-auto;
  }
  .preview :global(pre code) {
    @apply bg-transparent p-0 text-zinc-200;
  }
  .preview :global(a) {
    @apply text-indigo-400 underline;
  }
</style>
