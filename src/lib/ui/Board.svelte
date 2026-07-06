<script lang="ts">
  import { createEventDispatcher, onDestroy } from "svelte";
  import { fade } from "svelte/transition";
  import {
    XIcon,
    DownloadIcon,
    ZapIcon,
    LoaderIcon,
    CheckCircleIcon,
    AlertTriangleIcon,
    MaximizeIcon,
    PaperclipIcon,
    ImageIcon,
    VideoIcon,
  } from "svelte-feather-icons";

  import { slide } from "../action/slide";
  import { computeSnap, type SnapRect } from "../snap";
  import type { BoardItem } from "../protocol";
  import { parseJobPayload as parseJob, type JobPayload } from "../jobPayload";
  import { findGenModel, brandGroupsFor, modelsFor, type GenModel } from "../genModels";

  /** Board items (images + screen-share placeholder tiles). */
  export let items: BoardItem[];
  /** Live screen-share frame object URLs, keyed by item id. */
  export let streamSrcs: Record<string, string>;
  export let center: number[];
  export let zoom: number;
  export let hasWriteAccess: boolean | undefined;

  // Fabric offset CSS, shared with the terminal layer so tiles live in the same
  // infinite canvas and pan/zoom together.
  export let offsetLeftCss: string;
  export let offsetTopCss: string;
  export let offsetTransformOriginCss: string;

  /** Maps a pointer event to world-grid coordinates (same basis as terminals). */
  export let normalizePosition: (event: MouseEvent) => [number, number];
  /** Other rects (terminals + items) to soft-snap against, in world units. */
  export let snapTargets: SnapRect[] = [];
  /** Guide lines from a terminal drag (Session), rendered on the same fabric. */
  export let extraGuidesV: number[] = [];
  export let extraGuidesH: number[] = [];
  /** Shell (terminal) id currently under an asset drag, for Session's own
   * highlight — shells are Session-owned DOM, outside this component, so we
   * can only report the id up rather than binding a wrapper dict like
   * jobTileWrappers below. `null` when no asset drag is in flight or the
   * pointer isn't over a shell. */
  export let dropTargetShellId: number | null = null;

  // Screen-space snap distance; converted to world units (÷ zoom) so the pull
  // feels the same regardless of how far the board is zoomed.
  const SNAP_PX = 8;
  // Active guide lines (world coords) while dragging.
  let guidesV: number[] = [];
  let guidesH: number[] = [];

  // dottodot parity: full-size image/video viewer, opened from any
  // image/video tile's 🔍 button or a job node's result thumbnail.
  let lightboxSrc: string | null = null;
  let lightboxIsVideo = false;
  function openLightbox(src: string, isVideo = false) {
    lightboxSrc = src;
    lightboxIsVideo = isVideo;
  }
  function closeLightbox() {
    lightboxSrc = null;
    lightboxIsVideo = false;
  }

  const dispatch = createEventDispatcher<{
    move: { id: string; x: number; y: number };
    resize: { id: string; w: number; h: number };
    delete: string;
    edit: { id: string; text: string };
    jobEdit: {
      id: string;
      prompt?: string;
      model?: string;
      aspect_ratio?: string;
      resolution?: string;
      provider?: string;
      media_type?: "image" | "video";
      duration?: number;
      negative_prompt?: string;
    };
    jobGenerate: string;
    jobRetry: string;
    /** A reference image (dragged from another board tile) was dropped onto this job node. */
    jobAddImageRef: { id: string; imageItemId: string };
    /** Remove one reference image by index. */
    jobRemoveImageRef: { id: string; index: number };
    /** An i2v end-frame reference (dragged from an image tile) was dropped onto this job node. */
    jobSetEndFrame: { id: string; imageItemId: string };
    jobClearEndFrame: string;
    /** An image/video/job asset was dropped onto an agent's terminal (Session-owned shell). */
    assetDropOnShell: { assetItemId: string; shellId: number };
  }>();

  // ── Gen job node (image or video) — dataUrl is a JSON payload, not raw
  // text (see docs/round2-frontend-design.md, docs/vision-round3-gen-nodes-
  // design.md). Parsing lives in ../jobPayload (shared with Session.svelte)
  // — a malformed/partial dataUrl must never crash the render loop.
  //
  // Field names/values MUST match tools/board-bridge.ts exactly (it's the
  // live consumer, not a spec we control): `state`, not `status`; a *draft*
  // job must carry an explicit `state: "draft"` — the bridge treats a
  // MISSING state as immediately processable ("pending"-equivalent), so
  // omitting it while the user is still typing would make the bridge start
  // generating on every keystroke.
  //
  // "fal"/"novita" show in the UI ahead of bridge support (Le: "เดี๋ยว bridge
  // ผมตามรองรับ") — picking them is harmless today, bridge currently only
  // implements "kie" and ignores the field entirely otherwise.
  const PROVIDERS = ["kie", "fal", "novita"];

  // Resize state — drag the bottom-right handle to stretch a tile (like a window).
  const MIN_W = 120;
  const MIN_H = 80;
  let resizeId: string | null = null;
  let resizePointerId: number | null = null;
  let resizeStartW = 0;
  let resizeStartH = 0;
  let resizeStartWorld = [0, 0];
  let resizePending = false;
  let latestResizeEvent: PointerEvent | null = null;

  function startResize(event: PointerEvent, item: BoardItem) {
    if (hasWriteAccess === false) return;
    event.preventDefault();
    event.stopPropagation();
    resizeId = item.id;
    resizePointerId = event.pointerId ?? null;
    resizeStartW = item.w || MIN_W;
    resizeStartH = item.h || MIN_H;
    resizeStartWorld = normalizePosition(event);
    window.addEventListener("pointermove", onResize);
    window.addEventListener("pointerup", endResize);
    window.addEventListener("pointercancel", endResize);
  }

  function currentResize(event: PointerEvent) {
    const [wx, wy] = normalizePosition(event);
    const w = Math.max(MIN_W, Math.round(resizeStartW + (wx - resizeStartWorld[0])));
    const h = Math.max(MIN_H, Math.round(resizeStartH + (wy - resizeStartWorld[1])));
    return { w, h };
  }

  function onResize(event: PointerEvent) {
    if (resizeId === null || event.pointerId !== resizePointerId) return;
    latestResizeEvent = event; // use the freshest sample inside the frame
    if (!resizePending) {
      resizePending = true;
      requestAnimationFrame(() => {
        resizePending = false;
        if (resizeId !== null && latestResizeEvent) {
          const { w, h } = currentResize(latestResizeEvent);
          dispatch("resize", { id: resizeId, w, h });
        }
      });
    }
  }

  function endResize(event: PointerEvent) {
    if (event.pointerId !== resizePointerId) return;
    if (resizeId !== null) {
      const { w, h } = currentResize(event);
      dispatch("resize", { id: resizeId, w, h });
    }
    resizeId = null;
    resizePointerId = null;
    latestResizeEvent = null;
    window.removeEventListener("pointermove", onResize);
    window.removeEventListener("pointerup", endResize);
    window.removeEventListener("pointercancel", endResize);
  }

  // ── Asset drag (dottodot parity — "ลากรูปจากบอร์ดใส่ได้" — extended by
  // Vision Round 3 item 2 to video tiles + whole job nodes, and to a new
  // drop target: an agent's terminal, docs/vision-round3-asset-to-cli-
  // design.md): drag an image/video tile's 📎 handle (or a job node's own
  // handle) onto either a job node (i2i ref / end-frame) or a terminal
  // (asset-to-CLI). Image/job tiles render in this same `{#each items}`
  // loop, so job-node/end-frame hit-testing lives entirely here the same
  // way it always has (`jobTileWrappers`). Terminals are Session-owned DOM
  // outside this component — `document.elementFromPoint` finds them via a
  // `data-shell-id` attribute Session puts on its termWrapper div, rather
  // than sharing a wrapper dict across components.
  const jobTileWrappers: Record<string, HTMLDivElement> = {};
  // End-frame slots (Vision Round 3 — video i2v) are a smaller, more specific
  // drop target nested inside a job node — checked first so dropping exactly
  // on the end-frame box doesn't get swallowed by the whole-node ref target.
  const endFrameSlotWrappers: Record<string, HTMLDivElement> = {};
  let assetDragItemId: string | null = null;
  /** i2i ref / end-frame targets only make sense for images — video/job
   * drags skip straight to shell-hit-testing. */
  let assetDragKind: "image" | "video" | "job" | null = null;
  let assetDragPointerId: number | null = null;
  let dropTargetJobId: string | null = null;
  let dropTargetEndFrameId: string | null = null;

  function findJobUnderPoint(x: number, y: number): string | null {
    for (const [id, el] of Object.entries(jobTileWrappers)) {
      const r = el.getBoundingClientRect();
      if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return id;
    }
    return null;
  }

  function findEndFrameSlotUnderPoint(x: number, y: number): string | null {
    for (const [id, el] of Object.entries(endFrameSlotWrappers)) {
      const r = el.getBoundingClientRect();
      if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return id;
    }
    return null;
  }

  // Terminals live outside this component's DOM subtree (Session owns
  // `shells`/`termWrappers`) — hit-test the live DOM instead of a shared
  // wrapper dict. Safe because `elementFromPoint` resolves post-transform
  // screen coords, same basis `getBoundingClientRect()` above already uses.
  function findShellUnderPoint(x: number, y: number): number | null {
    const el = document.elementFromPoint(x, y) as HTMLElement | null;
    const wrapper = el?.closest<HTMLElement>("[data-shell-id]");
    const idStr = wrapper?.dataset.shellId;
    if (idStr === undefined) return null;
    const id = parseInt(idStr, 10);
    return isNaN(id) ? null : id;
  }

  function startAssetDrag(event: PointerEvent, itemId: string, kind: "image" | "video" | "job") {
    if (hasWriteAccess === false) return;
    event.preventDefault();
    event.stopPropagation();
    assetDragItemId = itemId;
    assetDragKind = kind;
    assetDragPointerId = event.pointerId ?? null;
    window.addEventListener("pointermove", onAssetDragMove);
    window.addEventListener("pointerup", endAssetDrag);
    window.addEventListener("pointercancel", cancelAssetDrag);
  }

  function onAssetDragMove(event: PointerEvent) {
    if (assetDragItemId === null || event.pointerId !== assetDragPointerId) return;
    const { clientX: x, clientY: y } = event;
    if (assetDragKind === "image") {
      dropTargetEndFrameId = findEndFrameSlotUnderPoint(x, y);
      dropTargetShellId = dropTargetEndFrameId ? null : findShellUnderPoint(x, y);
      dropTargetJobId = dropTargetEndFrameId || dropTargetShellId !== null ? null : findJobUnderPoint(x, y);
    } else {
      dropTargetEndFrameId = null;
      dropTargetJobId = null;
      dropTargetShellId = findShellUnderPoint(x, y);
    }
  }

  function endAssetDrag(event: PointerEvent) {
    if (assetDragItemId === null || event.pointerId !== assetDragPointerId) return;
    const { clientX: x, clientY: y } = event;
    const assetItemId = assetDragItemId;
    if (assetDragKind === "image") {
      const endFrameId = findEndFrameSlotUnderPoint(x, y);
      if (endFrameId) {
        dispatch("jobSetEndFrame", { id: endFrameId, imageItemId: assetItemId });
        cleanupAssetDrag();
        return;
      }
      const shellId = findShellUnderPoint(x, y);
      if (shellId !== null) {
        dispatch("assetDropOnShell", { assetItemId, shellId });
        cleanupAssetDrag();
        return;
      }
      const targetId = findJobUnderPoint(x, y);
      if (targetId) dispatch("jobAddImageRef", { id: targetId, imageItemId: assetItemId });
    } else {
      const shellId = findShellUnderPoint(x, y);
      if (shellId !== null) dispatch("assetDropOnShell", { assetItemId, shellId });
    }
    cleanupAssetDrag();
  }

  function cancelAssetDrag() {
    cleanupAssetDrag();
  }

  function cleanupAssetDrag() {
    assetDragItemId = null;
    assetDragKind = null;
    assetDragPointerId = null;
    dropTargetJobId = null;
    dropTargetEndFrameId = null;
    dropTargetShellId = null;
    window.removeEventListener("pointermove", onAssetDragMove);
    window.removeEventListener("pointerup", endAssetDrag);
    window.removeEventListener("pointercancel", cancelAssetDrag);
  }

  // Drag state. While dragging, the dragged tile renders at `dragPos` and sends
  // BoardMove on a requestAnimationFrame cadence (contract v2: client throttle).
  const LONG_PRESS_MS = 180;

  let dragId: string | null = null;
  let dragPointerId: number | null = null;
  let dragOffset = [0, 0];
  let dragPos = [0, 0];
  let rafPending = false;

  // Long-press state for mobile: hold ~400ms before drag activates.
  let pressTimer: ReturnType<typeof setTimeout> | null = null;
  let pressItem: BoardItem | null = null;
  let pressEvent: PointerEvent | null = null;
  let longPressActive = false;

  function onPointerDown(event: PointerEvent, item: BoardItem) {
    if (hasWriteAccess === false) return;
    // Keep the gesture on the item — don't let the canvas pan-handler also
    // claim it (caused touch drags to fight board panning).
    event.stopPropagation();
    // Desktop (mouse): start drag immediately.
    if (event.pointerType === "mouse") {
      startDrag(event, item);
      return;
    }
    // Touch: wait for long-press before allowing drag.
    if (pressTimer) {
      clearTimeout(pressTimer);
      window.removeEventListener("pointermove", onPressMove);
      window.removeEventListener("pointerup", cancelPress);
      window.removeEventListener("pointercancel", cancelPress);
    }
    pressItem = item;
    pressEvent = event;
    longPressActive = false;
    pressTimer = setTimeout(() => {
      if (pressItem && pressEvent) {
        // Hand off cleanly to the drag listeners — drop the press-phase ones so
        // a later pointerup doesn't fire both cancelPress and endDrag.
        window.removeEventListener("pointermove", onPressMove);
        window.removeEventListener("pointerup", cancelPress);
        window.removeEventListener("pointercancel", cancelPress);
        longPressActive = true;
        startDrag(pressEvent, pressItem);
      }
    }, LONG_PRESS_MS);
    window.addEventListener("pointermove", onPressMove);
    window.addEventListener("pointerup", cancelPress);
    window.addEventListener("pointercancel", cancelPress);
  }

  function onPressMove(event: PointerEvent) {
    if (longPressActive) {
      // Drag is active — the dedicated onMove listener handles movement.
      // Calling onMove here too double-processed every pointermove.
      return;
    }
    if (pressEvent && event.pointerId !== pressEvent.pointerId) return;
    // If finger moves too far before long-press fires, cancel.
    if (pressEvent) {
      const dx = event.clientX - pressEvent.clientX;
      const dy = event.clientY - pressEvent.clientY;
      if (dx * dx + dy * dy > 100) cancelPress(event);
    }
  }

  function cancelPress(event?: PointerEvent) {
    if (event && pressEvent && event.pointerId !== pressEvent.pointerId) return;
    if (pressTimer) clearTimeout(pressTimer);
    pressTimer = null;
    pressItem = null;
    pressEvent = null;
    if (longPressActive) {
      if (event) {
        endDrag(event);
      } else {
        dragId = null;
        dragPointerId = null;
        longPressActive = false;
        guidesV = [];
        guidesH = [];
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", endDrag);
        window.removeEventListener("pointercancel", endDrag);
      }
    }
    window.removeEventListener("pointermove", onPressMove);
    window.removeEventListener("pointerup", cancelPress);
    window.removeEventListener("pointercancel", cancelPress);
  }

  function startDrag(event: PointerEvent, item: BoardItem) {
    if (hasWriteAccess === false) return;
    event.preventDefault();
    const [wx, wy] = normalizePosition(event);
    dragId = item.id;
    dragPointerId = event.pointerId ?? null;
    dragOffset = [wx - item.x, wy - item.y];
    dragPos = [item.x, item.y];
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", endDrag);
    window.addEventListener("pointercancel", endDrag);
  }

  function onMove(event: PointerEvent) {
    if (dragId === null || event.pointerId !== dragPointerId) return;
    const [wx, wy] = normalizePosition(event);
    let nx = Math.round(wx - dragOffset[0]);
    let ny = Math.round(wy - dragOffset[1]);

    // Soft-snap this item's edges/center to other items + terminals, and light
    // up the guide lines we aligned to.
    const item = items.find((it) => it.id === dragId);
    if (item && snapTargets.length) {
      const others = snapTargets.filter((t) => t.id !== dragId);
      const r = computeSnap(nx, ny, item.w, item.h, others, SNAP_PX / zoom);
      nx = r.x;
      ny = r.y;
      guidesV = r.guidesV;
      guidesH = r.guidesH;
    }
    dragPos = [nx, ny];

    if (!rafPending) {
      rafPending = true;
      requestAnimationFrame(() => {
        rafPending = false;
        if (dragId !== null) {
          dispatch("move", { id: dragId, x: dragPos[0], y: dragPos[1] });
        }
      });
    }
  }

  function endDrag(event: PointerEvent) {
    if (event.pointerId !== dragPointerId) return;
    if (dragId !== null) {
      dispatch("move", { id: dragId, x: dragPos[0], y: dragPos[1] });
    }
    dragId = null;
    dragPointerId = null;
    longPressActive = false;
    guidesV = [];
    guidesH = [];
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", endDrag);
    window.removeEventListener("pointercancel", endDrag);
  }

  // Real download. A bare `<a download href={dataUrl}>` silently fails on mobile
  // (iOS Safari ignores `download` for data: URLs; Android blocks big ones), so
  // re-fetch into a Blob, hand out an object URL with a proper filename + ext,
  // and fall back to opening the raw URL (long-press-to-save) if that throws.
  async function downloadItem(item: BoardItem) {
    const ext = (mime: string) =>
      ({ "image/png": "png", "image/jpeg": "jpg", "image/webp": "webp", "image/gif": "gif", "video/mp4": "mp4", "video/webm": "webm" })[
        mime
      ] ?? (item.kind === "video" ? "mp4" : "png");
    try {
      const res = await fetch(item.dataUrl);
      if (!res.ok) throw new Error(`fetch ${res.status}`);
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${item.kind}-${item.id.slice(0, 6)}.${ext(blob.type)}`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      // Mobile share/save sheets can be slow — revoke late, not after 2s.
      setTimeout(() => URL.revokeObjectURL(url), 60_000);
    } catch {
      window.open(item.dataUrl, "_blank");
    }
  }

  onDestroy(() => {
    if (pressTimer) clearTimeout(pressTimer);
    window.removeEventListener("pointermove", onPressMove);
    window.removeEventListener("pointerup", cancelPress);
    window.removeEventListener("pointercancel", cancelPress);
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", endDrag);
    window.removeEventListener("pointercancel", endDrag);
    window.removeEventListener("pointermove", onResize);
    window.removeEventListener("pointerup", endResize);
    window.removeEventListener("pointercancel", endResize);
    window.removeEventListener("pointermove", onAssetDragMove);
    window.removeEventListener("pointerup", endAssetDrag);
    window.removeEventListener("pointercancel", cancelAssetDrag);
  });
</script>

{#each items.filter((it) => it.kind !== "doc" && it.kind !== "lock" && it.kind !== "label" && it.kind !== "link" && it.kind !== "asset-drop") as item (item.id)}
  {@const x = item.id === dragId ? dragPos[0] : item.x}
  {@const y = item.id === dragId ? dragPos[1] : item.y}
  <div
    class="absolute"
    style:left={offsetLeftCss}
    style:top={offsetTopCss}
    style:transform-origin={offsetTransformOriginCss}
    transition:fade|local
    use:slide={{ x, y, center, zoom, immediate: item.id === dragId }}
  >
    <div
      class="board-item"
      class:is-stream={item.kind === "stream"}
      class:is-note={item.kind === "note"}
      class:is-job={item.kind === "job"}
      style:width="{item.w}px"
      on:pointerdown={(event) => onPointerDown(event, item)}
    >
      {#if item.kind === "job"}
        {@const job = parseJob(item.dataUrl)}
        {@const editable = job.state === "draft"}
        {@const outItem = job.state === "done" ? items.find((it) => it.id === `${item.id}-out`) : undefined}
        {@const modelConfig = findGenModel(job.model)}
        {@const endFrameItem = job.end_frame_image_id ? items.find((it) => it.id === job.end_frame_image_id) : undefined}
        <div
          class="job-node"
          style:height="{item.h}px"
          class:is-drop-target={dropTargetJobId === item.id}
          bind:this={jobTileWrappers[item.id]}
        >
          <div class="job-head">
            <span class="job-title">{job.media_type === "video" ? "🎬 Video Gen" : "🎨 Image Gen"}</span>
            <div class="media-type-toggle">
              <button
                class="media-type-btn"
                class:selected={job.media_type !== "video"}
                title="Image"
                disabled={hasWriteAccess === false || !editable}
                on:pointerdown={(event) => event.stopPropagation()}
                on:click={() =>
                  dispatch("jobEdit", { id: item.id, media_type: "image", model: modelsFor("image")[0].id })}
              >
                <ImageIcon size="12" />
              </button>
              <button
                class="media-type-btn"
                class:selected={job.media_type === "video"}
                title="Video"
                disabled={hasWriteAccess === false || !editable}
                on:pointerdown={(event) => event.stopPropagation()}
                on:click={() =>
                  dispatch("jobEdit", { id: item.id, media_type: "video", model: modelsFor("video")[0].id })}
              >
                <VideoIcon size="12" />
              </button>
              {#if hasWriteAccess !== false}
                <!-- Vision Round 3 item 2: drag the whole node (prompt +
                     model, result too if done) onto an agent's terminal to
                     share it — disabled with no prompt yet, since an
                     empty-prompt hey is just noise (Le, PROGRESS.md
                     2026-07-07 01:56). -->
                <button
                  class="job-drag-handle"
                  title="Drag onto an agent's terminal to share this job"
                  disabled={!job.prompt.trim()}
                  on:pointerdown={(event) => {
                    event.stopPropagation();
                    startAssetDrag(event, item.id, "job");
                  }}
                >
                  <PaperclipIcon size="12" />
                </button>
              {/if}
            </div>
          </div>

          <div class="job-section">
            <span class="job-label">Model</span>
            <div class="model-grid">
              {#each brandGroupsFor(job.media_type) as group}
                {#each group.models as m}
                  <button
                    class="model-chip"
                    class:selected={job.model === m.id}
                    disabled={hasWriteAccess === false || !editable}
                    on:pointerdown={(event) => event.stopPropagation()}
                    on:click={() => dispatch("jobEdit", { id: item.id, model: m.id })}
                  >
                    <span class="model-name">{m.name}</span>
                    <span class="model-brand">{group.brand}</span>
                  </button>
                {/each}
              {/each}
            </div>
          </div>

          <textarea
            class="job-prompt"
            placeholder={job.media_type === "video" ? "Describe a video…" : "Describe the image…"}
            value={job.prompt}
            readonly={hasWriteAccess === false || !editable}
            on:pointerdown={(event) => event.stopPropagation()}
            on:input={(event) =>
              dispatch("jobEdit", {
                id: item.id,
                prompt: event.currentTarget.value,
              })}
          />

          {#if modelConfig?.hasNegativePrompt}
            <textarea
              class="job-prompt job-negative-prompt"
              placeholder="Negative prompt (things to avoid)…"
              value={job.negative_prompt ?? ""}
              readonly={hasWriteAccess === false || !editable}
              on:pointerdown={(event) => event.stopPropagation()}
              on:input={(event) =>
                dispatch("jobEdit", {
                  id: item.id,
                  negative_prompt: event.currentTarget.value,
                })}
            />
          {/if}

          <div class="job-section">
            <span class="job-label">Aspect Ratio</span>
            <div class="chip-row">
              {#each modelConfig?.aspectRatios ?? [] as ar}
                <button
                  class="chip"
                  class:selected={job.aspect_ratio === ar}
                  disabled={hasWriteAccess === false || !editable}
                  on:pointerdown={(event) => event.stopPropagation()}
                  on:click={() => dispatch("jobEdit", { id: item.id, aspect_ratio: ar })}
                >
                  {ar}
                </button>
              {/each}
            </div>
          </div>

          <div class="job-section">
            <span class="job-label">Resolution</span>
            <div class="chip-row">
              {#each modelConfig?.resolutions ?? [] as res}
                <button
                  class="chip"
                  class:selected={job.resolution === res}
                  disabled={hasWriteAccess === false || !editable}
                  on:pointerdown={(event) => event.stopPropagation()}
                  on:click={() => dispatch("jobEdit", { id: item.id, resolution: res })}
                >
                  {res}
                </button>
              {/each}
            </div>
          </div>

          {#if modelConfig?.durations?.length}
            <div class="job-section">
              <span class="job-label">Duration</span>
              <div class="chip-row">
                {#each modelConfig.durations as d}
                  <button
                    class="chip"
                    class:selected={job.duration === d}
                    disabled={hasWriteAccess === false || !editable}
                    on:pointerdown={(event) => event.stopPropagation()}
                    on:click={() => dispatch("jobEdit", { id: item.id, duration: d })}
                  >
                    {d}s
                  </button>
                {/each}
              </div>
            </div>
          {/if}

          <div class="job-section">
            <span class="job-label">Provider</span>
            <div class="chip-row">
              {#each PROVIDERS as p}
                <button
                  class="chip"
                  class:selected={job.provider === p}
                  disabled={hasWriteAccess === false || !editable}
                  on:pointerdown={(event) => event.stopPropagation()}
                  on:click={() => dispatch("jobEdit", { id: item.id, provider: p })}
                >
                  {p}
                </button>
              {/each}
            </div>
          </div>

          <div class="job-section">
            <span class="job-label">Reference images ({job.input_image_ids.length}) — drag an image tile here for i2i</span>
            <div class="ref-slots">
              {#each job.input_image_ids as refId, i (refId + i)}
                {@const refItem = items.find((it) => it.id === refId)}
                {#if refItem}
                  <div class="ref-thumb">
                    <img src={refItem.dataUrl} alt="reference" />
                    {#if editable}
                      <button
                        class="ref-remove"
                        on:pointerdown={(event) => event.stopPropagation()}
                        on:click={() => dispatch("jobRemoveImageRef", { id: item.id, index: i })}
                      >
                        <XIcon size="10" />
                      </button>
                    {/if}
                  </div>
                {/if}
              {/each}
              {#if job.input_image_ids.length === 0}
                <div class="ref-empty">no references yet</div>
              {/if}
            </div>
          </div>

          {#if modelConfig?.hasEndFrame}
            <div class="job-section">
              <span class="job-label">End frame — drag an image tile here</span>
              <div
                class="end-frame-slot"
                class:is-drop-target={dropTargetEndFrameId === item.id}
                bind:this={endFrameSlotWrappers[item.id]}
              >
                {#if endFrameItem}
                  <div class="ref-thumb">
                    <img src={endFrameItem.dataUrl} alt="end frame" />
                    {#if editable}
                      <button
                        class="ref-remove"
                        on:pointerdown={(event) => event.stopPropagation()}
                        on:click={() => dispatch("jobClearEndFrame", item.id)}
                      >
                        <XIcon size="10" />
                      </button>
                    {/if}
                  </div>
                {:else}
                  <div class="ref-empty">no end frame yet</div>
                {/if}
              </div>
            </div>
          {/if}

          <div class="job-footer">
            {#if job.state === "pending" || job.state === "running"}
              <div class="job-banner">
                <LoaderIcon size="14" class="spin" />
                <span>{job.state === "pending" ? "Queued — waiting for bridge…" : "Generating…"}</span>
              </div>
            {:else if job.state === "error"}
              <div class="job-banner job-banner-error">
                <AlertTriangleIcon size="14" />
                <span>{job.error ?? "Generation failed"}</span>
              </div>
              <button
                class="job-btn"
                on:pointerdown={(event) => event.stopPropagation()}
                on:click={() => dispatch("jobRetry", item.id)}
              >
                Retry
              </button>
            {:else if job.state === "done"}
              <div class="job-banner job-banner-done">
                <CheckCircleIcon size="14" />
                <span>Done</span>
              </div>
              {#if outItem}
                <button
                  class="job-result-thumb"
                  title="Open full size"
                  on:pointerdown={(event) => event.stopPropagation()}
                  on:click={() => openLightbox(outItem.dataUrl, outItem.kind === "video")}
                >
                  {#if outItem.kind === "video"}
                    <!-- svelte-ignore a11y-media-has-caption -->
                    <video src={outItem.dataUrl} muted playsinline />
                  {:else}
                    <img src={outItem.dataUrl} alt="result" />
                  {/if}
                </button>
              {/if}
              <button
                class="job-btn"
                on:pointerdown={(event) => event.stopPropagation()}
                on:click={() => dispatch("jobRetry", item.id)}
              >
                Generate again
              </button>
            {:else}
              <button
                class="job-btn job-btn-primary"
                disabled={hasWriteAccess === false || !job.prompt.trim()}
                on:pointerdown={(event) => event.stopPropagation()}
                on:click={() => dispatch("jobGenerate", item.id)}
              >
                <ZapIcon size="14" />
                <span>Generate</span>
              </button>
            {/if}
          </div>
        </div>
      {:else if item.kind === "note"}
        <textarea
          class="note-text"
          style:height="{item.h}px"
          placeholder="Type a note…"
          value={item.dataUrl}
          readonly={hasWriteAccess === false}
          on:pointerdown={(event) => event.stopPropagation()}
          on:input={(event) =>
            dispatch("edit", {
              id: item.id,
              text: event.currentTarget.value,
            })}
        />
      {:else if item.kind === "video"}
        <!-- svelte-ignore a11y-media-has-caption -->
        <video
          src={item.dataUrl}
          style:width="{item.w}px"
          controls
          playsinline
          on:pointerdown={(event) => event.stopPropagation()}
        />
      {:else}
        <img
          src={streamSrcs[item.id] ?? item.dataUrl}
          alt={item.kind === "stream" ? "screen share" : "shared image"}
          draggable="false"
        />
      {/if}

      {#if item.kind === "stream"}
        <div class="live-tag">● LIVE</div>
      {/if}

      {#if item.kind === "video" || item.kind === "image"}
        <button
          class="download"
          title="Download {item.kind}"
          on:pointerdown={(event) => event.stopPropagation()}
          on:click={() => downloadItem(item)}
        >
          <DownloadIcon size="14" />
        </button>
        <button
          class="lightbox-open"
          title="Open full size"
          on:pointerdown={(event) => event.stopPropagation()}
          on:click={() => openLightbox(streamSrcs[item.id] ?? item.dataUrl, item.kind === "video")}
        >
          <MaximizeIcon size="14" />
        </button>
      {/if}

      {#if (item.kind === "image" || item.kind === "video") && hasWriteAccess !== false}
        <!-- dottodot parity: "ลากรูปจากบอร์ดใส่ได้" — drag an image onto a job
             node's reference-images/end-frame section to use as input, or
             drag either onto an agent's terminal to share it (Vision Round 3
             item 2, docs/vision-round3-asset-to-cli-design.md). -->
        <button
          class="ref-handle"
          title={item.kind === "image"
            ? "Drag onto a job node to use as a reference, or an agent's terminal to share it"
            : "Drag onto an agent's terminal to share it"}
          on:pointerdown={(event) => startAssetDrag(event, item.id, item.kind === "video" ? "video" : "image")}
        >
          <PaperclipIcon size="12" />
        </button>
      {/if}

      {#if hasWriteAccess !== false}
        <button
          class="drag-grip"
          title="Drag to move"
          on:pointerdown={(event) => {
            event.stopPropagation();
            startDrag(event, item);
          }}
        >
          ⠿
        </button>
        <button
          class="delete"
          title="Remove"
          on:pointerdown={(event) => event.stopPropagation()}
          on:click={() => dispatch("delete", item.id)}
        >
          <XIcon size="14" />
        </button>
        <!-- Resize handle: drag to stretch the tile like a window. -->
        <div
          class="resize-handle"
          class:resizing={resizeId === item.id}
          title="Drag to resize"
          on:pointerdown={(event) => startResize(event, item)}
        />
      {/if}
    </div>
  </div>
{/each}

<!-- Alignment guides: thin lines through the world coords we snapped to. They
     ride the same canvas fabric (slide) as the items so they track pan/zoom. -->
{#each [...guidesV, ...extraGuidesV] as gx}
  <div
    class="absolute pointer-events-none"
    style:left={offsetLeftCss}
    style:top={offsetTopCss}
    style:transform-origin={offsetTransformOriginCss}
    use:slide={{ x: gx, y: center[1], center, zoom, immediate: true }}
  >
    <div class="guide guide-v" />
  </div>
{/each}
{#each [...guidesH, ...extraGuidesH] as gy}
  <div
    class="absolute pointer-events-none"
    style:left={offsetLeftCss}
    style:top={offsetTopCss}
    style:transform-origin={offsetTransformOriginCss}
    use:slide={{ x: center[0], y: gy, center, zoom, immediate: true }}
  >
    <div class="guide guide-h" />
  </div>
{/each}

<!-- dottodot parity: full-size image/video viewer. `fixed` (not part of the
     pan/zoom fabric) so it always centers on the real viewport. -->
{#if lightboxSrc}
  <div
    class="lightbox-backdrop"
    role="button"
    tabindex="0"
    on:click={closeLightbox}
    on:keydown={(event) => event.key === "Escape" && closeLightbox()}
  >
    {#if lightboxIsVideo}
      <!-- svelte-ignore a11y-media-has-caption -->
      <video src={lightboxSrc} controls autoplay class="lightbox-img" />
    {:else}
      <img src={lightboxSrc} alt="full size preview" class="lightbox-img" />
    {/if}
    <button class="lightbox-close" on:click={closeLightbox}>
      <XIcon size="20" />
    </button>
  </div>
{/if}

<style lang="postcss">
  .board-item {
    @apply relative rounded-lg overflow-hidden bg-zinc-900 shadow-lg cursor-move select-none;
    @apply ring-1 ring-zinc-700 transition-transform duration-150;
  }

  /* Snap guide lines — span well past the viewport so they read as full-length
     rules; width/height in world px (the slide transform scales them by zoom). */
  .guide {
    @apply absolute pointer-events-none;
    background: theme("colors.indigo.400");
  }
  .guide-v {
    width: 1px;
    height: 6000px;
    top: -3000px;
    left: 0;
  }
  .guide-h {
    height: 1px;
    width: 6000px;
    left: -3000px;
    top: 0;
  }

  .board-item:active {
    @apply scale-[1.02];
  }

  .board-item.is-stream {
    @apply ring-2 ring-red-500/70;
  }

  .board-item.is-note {
    @apply bg-amber-200 ring-amber-300/60 shadow-amber-900/30;
  }

  .note-text {
    @apply block w-full p-3 bg-transparent resize-none outline-none border-0;
    @apply text-sm text-amber-950 placeholder-amber-700/50 font-medium leading-snug;
    cursor: text;
  }

  .board-item.is-job {
    @apply ring-indigo-400/60 shadow-indigo-900/30;
  }

  .job-node {
    @apply flex flex-col gap-2.5 p-2.5 bg-zinc-900 cursor-default overflow-y-auto;
  }

  .job-head {
    @apply flex items-center justify-between gap-2;
  }

  .job-title {
    @apply text-xs font-semibold text-zinc-300;
  }

  .job-prompt {
    @apply flex-1 min-h-[100px] p-2 rounded-md bg-zinc-800/80 resize-none outline-none border-0;
    @apply text-sm text-zinc-100 placeholder-zinc-500 leading-snug ring-1 ring-zinc-700;
    @apply focus:ring-indigo-500;
    cursor: text;
  }

  .job-footer {
    @apply flex items-center gap-2 min-h-[28px];
  }

  /* Pill-style status banner (dottodot's JobStatusBanner pattern) — icon +
     text in a tinted, bordered chip instead of a bare status line. */
  .job-banner {
    @apply flex items-center gap-1.5 flex-1 min-w-0 px-2 py-1 rounded-md text-xs;
    @apply text-indigo-300 bg-indigo-500/10 ring-1 ring-indigo-500/30;
  }

  .job-banner span {
    @apply truncate;
  }

  .job-banner-error {
    @apply text-red-300 bg-red-500/10 ring-red-500/30;
  }

  .job-banner-done {
    @apply text-emerald-300 bg-emerald-500/10 ring-emerald-500/30;
  }

  :global(.job-banner .spin) {
    @apply animate-spin flex-none;
  }

  .job-btn {
    @apply text-xs px-2.5 py-1 rounded-md bg-zinc-700/80 text-zinc-200 hover:bg-zinc-600 flex-none;
  }

  .job-btn-primary {
    @apply flex items-center justify-center gap-1.5 bg-indigo-600 text-white hover:bg-indigo-500 ml-auto;
    @apply disabled:opacity-40 disabled:hover:bg-indigo-600 disabled:cursor-not-allowed;
  }

  /* dottodot parity sections (model/aspect-ratio/resolution/provider/refs) */
  .job-node.is-drop-target {
    @apply ring-2 ring-amber-400;
  }

  .job-section {
    @apply flex flex-col gap-1;
  }

  .job-label {
    @apply text-[10px] uppercase tracking-wide text-zinc-500;
  }

  .model-grid {
    @apply grid grid-cols-2 gap-1;
  }

  .model-chip {
    @apply flex flex-col items-start px-2 py-1 rounded-md bg-zinc-800 ring-1 ring-zinc-700;
    @apply hover:ring-indigo-500 disabled:opacity-40 disabled:cursor-not-allowed text-left;
  }

  .model-chip.selected {
    @apply bg-indigo-500/20 ring-indigo-500;
  }

  .model-name {
    @apply text-xs text-zinc-100 leading-tight;
  }

  .model-brand {
    @apply text-[9px] text-zinc-500 leading-tight;
  }

  .chip-row {
    @apply flex flex-wrap gap-1;
  }

  .chip {
    @apply text-[11px] px-2 py-0.5 rounded-full bg-zinc-800 text-zinc-300 ring-1 ring-zinc-700;
    @apply hover:ring-indigo-500 disabled:opacity-40 disabled:cursor-not-allowed;
  }

  .chip.selected {
    @apply bg-indigo-500/20 ring-indigo-500 text-indigo-200;
  }

  .ref-slots {
    @apply flex flex-wrap gap-1.5 min-h-[44px] p-1.5 rounded-md bg-zinc-800/50 border border-dashed border-zinc-700;
  }

  .ref-thumb {
    @apply relative w-10 h-10 rounded overflow-hidden ring-1 ring-zinc-700;
  }

  .ref-thumb img {
    @apply w-full h-full object-cover;
  }

  .ref-remove {
    @apply absolute top-0 right-0 p-0.5 bg-black/60 text-white rounded-bl;
  }

  .ref-empty {
    @apply flex items-center text-[11px] text-zinc-600 px-1;
  }

  .job-result-thumb {
    @apply block w-full max-h-24 rounded-md overflow-hidden ring-1 ring-zinc-700 flex-none;
  }

  .job-result-thumb img,
  .job-result-thumb video {
    @apply w-full h-full object-cover;
  }

  /* Vision Round 3 — media-type toggle (image/video) in the job-node head. */
  .media-type-toggle {
    @apply flex gap-0.5 flex-none;
  }

  .media-type-btn {
    @apply p-1 rounded bg-zinc-800 text-zinc-500 ring-1 ring-zinc-700;
    @apply hover:text-zinc-300 disabled:opacity-40 disabled:cursor-not-allowed;
  }

  .media-type-btn.selected {
    @apply bg-indigo-500/20 ring-indigo-500 text-indigo-200;
  }

  .job-drag-handle {
    @apply p-1 ml-1 rounded bg-zinc-800 text-zinc-500 ring-1 ring-zinc-700 flex-none;
    @apply hover:bg-amber-600 hover:text-white cursor-grab touch-none disabled:opacity-30;
    @apply disabled:cursor-not-allowed disabled:hover:bg-zinc-800 disabled:hover:text-zinc-500;
  }

  .job-negative-prompt {
    @apply min-h-[44px] text-xs;
  }

  .end-frame-slot {
    @apply flex items-center gap-1.5 min-h-[44px] p-1.5 rounded-md bg-zinc-800/50 border border-dashed border-zinc-700;
  }

  .end-frame-slot.is-drop-target {
    @apply ring-2 ring-amber-400 border-amber-400;
  }

  .board-item img {
    @apply block w-full h-auto pointer-events-none;
  }

  .board-item video {
    @apply block w-full h-auto bg-black;
  }

  .live-tag {
    @apply absolute top-1.5 left-1.5 text-[10px] font-semibold tracking-wide;
    @apply px-1.5 py-0.5 rounded bg-red-600 text-white pointer-events-none;
  }

  .delete {
    @apply absolute top-1 right-1 z-30 p-0.5 rounded bg-zinc-800/80 text-zinc-300;
    @apply opacity-0 transition-opacity hover:bg-red-600 hover:text-white;
  }

  .board-item:hover .delete {
    @apply opacity-100;
  }

  .download {
    @apply absolute bottom-1 left-1 p-0.5 rounded bg-zinc-800/80 text-zinc-300 z-30;
    @apply opacity-0 transition-opacity hover:bg-indigo-600 hover:text-white;
  }

  .lightbox-open {
    @apply absolute bottom-1 left-8 p-0.5 rounded bg-zinc-800/80 text-zinc-300 z-30;
    @apply opacity-0 transition-opacity hover:bg-indigo-600 hover:text-white;
  }

  .ref-handle {
    @apply absolute bottom-1 left-[60px] p-0.5 rounded bg-zinc-800/80 text-zinc-300 z-30;
    @apply opacity-0 transition-opacity hover:bg-amber-600 hover:text-white cursor-grab touch-none;
  }

  .drag-grip {
    @apply absolute top-1 left-1 z-20 rounded bg-zinc-800/80 text-zinc-300;
    @apply px-1.5 py-0.5 text-xs leading-none cursor-move touch-none select-none;
    @apply opacity-0 transition-opacity;
  }

  .board-item:hover .drag-grip {
    @apply opacity-100;
  }

  .board-item:hover .download,
  .board-item:hover .lightbox-open,
  .board-item:hover .ref-handle {
    @apply opacity-100;
  }

  .resize-handle {
    @apply absolute bottom-0 right-0 w-4 h-4 z-20 cursor-nwse-resize touch-none;
    @apply opacity-0 transition-opacity;
    background: linear-gradient(
      135deg,
      transparent 0%,
      transparent 50%,
      theme("colors.indigo.400") 50%,
      theme("colors.indigo.400") 100%
    );
    border-bottom-right-radius: theme("borderRadius.lg");
  }

  .board-item:hover .resize-handle,
  .resize-handle.resizing {
    @apply opacity-90;
  }

  /* Touch devices have no hover — keep board-item controls visible + finger-sized. */
  @media (hover: none), (pointer: coarse) {
    .delete,
    .download,
    .lightbox-open,
    .ref-handle,
    .drag-grip {
      @apply opacity-100 p-1.5;
    }
    .resize-handle {
      @apply opacity-90 w-11 h-11;
    }
    .drag-grip {
      @apply min-w-[44px] min-h-[44px] grid place-items-center text-base;
    }
  }

  .lightbox-backdrop {
    @apply fixed inset-0 z-[100] flex items-center justify-center bg-black/85 p-8;
  }

  .lightbox-img {
    @apply max-w-full max-h-full rounded-lg shadow-2xl;
  }

  .lightbox-close {
    @apply absolute top-4 right-4 p-2 rounded-full bg-zinc-800/80 text-zinc-200 hover:bg-red-600 hover:text-white;
  }
</style>
