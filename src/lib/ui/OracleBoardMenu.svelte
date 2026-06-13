<!--
  Oracle Board dropdown — a conky-style system monitor that drops down from the
  toolbar brand. Polls /api/sysstat (same source as the StatusBar) only while open.
-->
<script lang="ts">
  import { onDestroy } from "svelte";

  export let open = false;

  let cpu = 0;
  let memPct = 0;
  let memUsedMb = 0;
  let memTotalMb = 0;
  let temp: number | null = null;
  let load = 0;
  let ok = false;
  let timer: ReturnType<typeof setInterval> | null = null;

  // arra (knowledge base) status — written to /arra-stats.json by a cron writer.
  let arra: {
    ok: boolean;
    total: number;
    byType: Record<string, number>;
    indexAgeHours: number | null;
    dbMb: number;
  } | null = null;

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
    try {
      const ar = await fetch("/arra-stats.json", { cache: "no-store" });
      arra = ar.ok ? await ar.json() : null;
    } catch {
      arra = null;
    }
  }

  // Poll on a 2s cadence only while the menu is open.
  $: if (open && timer === null) {
    poll();
    timer = setInterval(poll, 2000);
  } else if (!open && timer !== null) {
    clearInterval(timer);
    timer = null;
  }
  onDestroy(() => timer && clearInterval(timer));

  function level(pct: number): string {
    if (pct >= 85) return "bar-red";
    if (pct >= 60) return "bar-amber";
    return "bar-green";
  }
  const gb = (mb: number) => (mb / 1024).toFixed(1);
</script>

{#if open}
  <div class="board-menu">
    <div class="head">◉ ORACLE BOARD · system</div>

    <div class="metric">
      <div class="line"><span class="lbl">CPU</span><span class="val">{cpu}%</span></div>
      <div class="track"><div class="fill {level(cpu)}" style:width="{cpu}%" /></div>
    </div>

    <div class="metric">
      <div class="line">
        <span class="lbl">RAM</span><span class="val">{gb(memUsedMb)}/{gb(memTotalMb)} GB</span>
      </div>
      <div class="track"><div class="fill {level(memPct)}" style:width="{memPct}%" /></div>
    </div>

    {#if temp !== null}
      <div class="metric">
        <div class="line"><span class="lbl">TEMP</span><span class="val">{temp}°C</span></div>
        <div class="track">
          <div class="fill {level(temp)}" style:width="{Math.min(temp, 100)}%" />
        </div>
      </div>
    {/if}

    <div class="foot">
      <span class="lbl">LOAD</span><span class="val">{load}</span>
      <span class="dot" class:online={ok} />
    </div>

    <!-- arra knowledge base -->
    <div class="arra">
      <div class="head2">
        <span>◆ ARRA · knowledge</span>
        <span class="dot" class:online={arra?.ok} />
      </div>
      {#if arra}
        <div class="row"><span class="lbl">DOCS</span><span class="val">{arra.total.toLocaleString()}</span></div>
        <div class="row">
          <span class="lbl">DB</span><span class="val">{(arra.dbMb / 1024).toFixed(1)} GB</span>
        </div>
        <div class="row">
          <span class="lbl">INDEX</span>
          <span class="val">{arra.indexAgeHours != null ? `${arra.indexAgeHours.toFixed(1)}h ago` : "—"}</span>
        </div>
        {#if arra.byType}
          <div class="types">
            {#each Object.entries(arra.byType) as [k, v]}
              <span class="chip">{k} {v.toLocaleString()}</span>
            {/each}
          </div>
        {/if}
      {:else}
        <div class="row"><span class="lbl">arra</span><span class="val">offline</span></div>
      {/if}
    </div>
  </div>
{/if}

<style lang="postcss">
  .board-menu {
    @apply absolute left-0 top-full mt-2 z-40 w-60 max-w-[calc(100vw-1rem)] p-3 rounded-xl;
    @apply bg-zinc-900/95 ring-1 ring-zinc-700 shadow-2xl backdrop-blur-sm;
    @apply font-mono text-xs text-zinc-300 select-none;
  }
  .head {
    @apply text-[10px] tracking-widest text-indigo-400 mb-2.5;
  }
  .metric {
    @apply mb-2.5;
  }
  .line {
    @apply flex items-center justify-between mb-1;
  }
  .lbl {
    @apply text-[10px] text-zinc-500;
  }
  .val {
    @apply tabular-nums text-zinc-200;
  }
  .track {
    @apply h-1.5 rounded-full bg-zinc-800 overflow-hidden;
  }
  .fill {
    @apply h-full rounded-full transition-all duration-300;
  }
  .bar-green {
    @apply bg-emerald-500;
  }
  .bar-amber {
    @apply bg-amber-500;
  }
  .bar-red {
    @apply bg-red-500;
  }
  .foot {
    @apply flex items-center gap-2 mt-1 pt-2 border-t border-zinc-800;
  }
  .dot {
    @apply ml-auto w-2 h-2 rounded-full bg-zinc-600;
  }
  .dot.online {
    @apply bg-emerald-500;
  }
  .arra {
    @apply mt-3 pt-2.5 border-t border-zinc-800;
  }
  .head2 {
    @apply flex items-center text-[10px] tracking-widest text-fuchsia-400 mb-2;
  }
  .head2 .dot {
    @apply ml-auto;
  }
  .row {
    @apply flex items-center justify-between mb-1;
  }
  .types {
    @apply flex flex-wrap gap-1 mt-1.5;
  }
  .chip {
    @apply px-1.5 py-0.5 rounded bg-zinc-800 text-[10px] text-zinc-400;
  }
</style>
