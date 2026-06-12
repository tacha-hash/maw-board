<!--
  Live system status bar (CPU / RAM / temperature) for the toolbar — a small
  conky-style readout polled from the server's /api/sysstat endpoint.
-->
<script lang="ts">
  import { onMount, onDestroy } from "svelte";

  let cpu = 0;
  let memPct = 0;
  let memUsedMb = 0;
  let memTotalMb = 0;
  let temp: number | null = null;
  let load = 0;
  let ok = false;
  let timer: ReturnType<typeof setInterval>;

  async function poll() {
    try {
      const r = await fetch("/api/sysstat", { cache: "no-store" });
      if (!r.ok) return;
      const d = await r.json();
      cpu = d.cpu ?? 0;
      memPct = d.memPct ?? 0;
      memUsedMb = d.memUsedMb ?? 0;
      memTotalMb = d.memTotalMb ?? 0;
      temp = d.temp ?? null;
      load = d.load ?? 0;
      ok = true;
    } catch {
      ok = false;
    }
  }

  // Color a metric green → amber → red as it climbs.
  function level(pct: number): string {
    if (pct >= 85) return "text-red-400";
    if (pct >= 60) return "text-amber-400";
    return "text-emerald-400";
  }

  function gb(mb: number): string {
    return (mb / 1024).toFixed(1);
  }

  onMount(() => {
    poll();
    timer = setInterval(poll, 3000);
  });
  onDestroy(() => clearInterval(timer));
</script>

{#if ok}
  <div class="statusbar" title={`load ${load} · RAM ${gb(memUsedMb)}/${gb(memTotalMb)} GB`}>
    <span class="metric"><span class="lbl">CPU</span><span class={level(cpu)}>{cpu}%</span></span>
    <span class="sep">·</span>
    <span class="metric"><span class="lbl">RAM</span><span class={level(memPct)}>{memPct}%</span></span>
    {#if temp !== null}
      <span class="sep">·</span>
      <span class="metric"><span class="lbl">TEMP</span><span class={level(temp)}>{temp}°</span></span>
    {/if}
  </div>
{/if}

<style lang="postcss">
  .statusbar {
    @apply flex items-center gap-1.5 px-2 text-xs font-mono tabular-nums select-none;
    @apply text-zinc-400;
  }
  .metric {
    @apply flex items-center gap-1;
  }
  .lbl {
    @apply text-[10px] text-zinc-500;
  }
  .sep {
    @apply text-zinc-700;
  }
</style>
