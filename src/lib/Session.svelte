<script lang="ts">
  import {
    onDestroy,
    onMount,
    tick,
    beforeUpdate,
    afterUpdate,
    createEventDispatcher,
  } from "svelte";
  import { fade } from "svelte/transition";
  import { debounce, throttle } from "lodash-es";

  import { Encrypt } from "./encrypt";
  import { createLock } from "./lock";
  import { Srocket } from "./srocket";
  import type {
    BoardItem,
    WsClient,
    WsServer,
    WsUser,
    WsWinsize,
  } from "./protocol";
  import {
    playVoice,
    readImageFile,
    startScreenShare,
    type StreamController,
  } from "./board";
  import { RtcMesh } from "./rtc";
  import { makeToast } from "./toast";
  import {
    computeSnap,
    computeSnapTarget,
    detectEdgeSnapAction,
    applySnapGap,
    isSnapAction,
    snapShortcutAction,
    snapSharedEdges,
    type SnapRect,
    type SnapAction,
    type SnapShortcutAction,
    type ViewRect,
  } from "./snap";
  import Board from "./ui/Board.svelte";
  import Chat, { type ChatMessage } from "./ui/Chat.svelte";
  import ChooseName from "./ui/ChooseName.svelte";
  import NameList from "./ui/NameList.svelte";
  import NetworkInfo from "./ui/NetworkInfo.svelte";
  import Settings from "./ui/Settings.svelte";
  import Toolbar from "./ui/Toolbar.svelte";
  import XTerm from "./ui/XTerm.svelte";
  import CameraPreview from "./ui/CameraPreview.svelte";
  import FileExplorer from "./ui/FileExplorer.svelte";
  import MarkdownDoc from "./ui/MarkdownDoc.svelte";
  import Numpad from "./ui/Numpad.svelte";
  import SnippetBar from "./ui/SnippetBar.svelte";
  import YouTubePopup from "./ui/YouTubePopup.svelte";
  import Avatars from "./ui/Avatars.svelte";
  import LiveCursor from "./ui/LiveCursor.svelte";
  import { slide } from "./action/slide";
  import { TouchZoom, INITIAL_ZOOM } from "./action/touchZoom";
  import { arrangeNewTerminal } from "./arrange";
  import { settings } from "./settings";
  import { EyeIcon } from "svelte-feather-icons";

  export let id: string;

  const dispatch = createEventDispatcher<{ receiveName: string }>();

  // The magic numbers "left" and "top" are used to approximately center the
  // terminal at the time that it is first created.
  const CONSTANT_OFFSET_LEFT = 378;
  const CONSTANT_OFFSET_TOP = 240;

  const OFFSET_LEFT_CSS = `calc(50vw - ${CONSTANT_OFFSET_LEFT}px)`;
  const OFFSET_TOP_CSS = `calc(50vh - ${CONSTANT_OFFSET_TOP}px)`;
  const OFFSET_TRANSFORM_ORIGIN_CSS = `calc(-1 * ${OFFSET_LEFT_CSS}) calc(-1 * ${OFFSET_TOP_CSS})`;

  // Terminal width and height limits.
  const TERM_MIN_ROWS = 8;
  const TERM_MIN_COLS = 32;
  const TERM_MAX_ROWS = 200;
  const TERM_MAX_COLS = 500;
  const DEFAULT_PORTRAIT_COLS = 60;
  const DEFAULT_PORTRAIT_ROWS = 52;
  const PENDING_CREATE_TTL_MS = 10000;

  function getConstantOffset() {
    return [
      0.5 * window.innerWidth - CONSTANT_OFFSET_LEFT,
      0.5 * window.innerHeight - CONSTANT_OFFSET_TOP,
    ];
  }

  let fabricEl: HTMLElement;
  let touchZoom: TouchZoom;
  let center = [0, 0];
  let zoom = INITIAL_ZOOM;

  let showChat = false; // @hmr:keep
  let settingsOpen = false; // @hmr:keep
  let showExplorer = false; // @hmr:keep
  let showDoc = false; // @hmr:keep
  let showSnippets = false; // @hmr:keep
  let showYouTube = false; // @hmr:keep
  let showNumpad = false; // @hmr:keep

  // Auto-hiding toolbar (Apple menu-bar style): fades out after inactivity,
  // reveals when the pointer nears the top edge or hovers it.
  let toolbarVisible = true;
  let toolbarHideTimer: ReturnType<typeof setTimeout>;
  const TOOLBAR_HIDE_MS = 12000; // Bo 2026-06-13: 3s felt too rushed
  function showToolbar() {
    toolbarVisible = true;
    clearTimeout(toolbarHideTimer);
    toolbarHideTimer = setTimeout(() => (toolbarVisible = false), TOOLBAR_HIDE_MS);
  }
  onMount(() => {
    const onMove = (e: PointerEvent) => {
      if (e.clientY < 96) showToolbar();
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerdown", onMove);
    showToolbar(); // visible on load, then auto-hide
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerdown", onMove);
      clearTimeout(toolbarHideTimer);
    };
  });
  let showNetworkInfo = false; // @hmr:keep

  onMount(() => {
    touchZoom = new TouchZoom(fabricEl);
    touchZoom.onMove(() => {
      center = touchZoom.center;
      zoom = touchZoom.zoom;

      // Blur if the user is currently focused on a terminal.
      //
      // This makes it so that panning does not stop when the cursor happens to
      // intersect with the textarea, which absorbs wheel and touch events.
      if (document.activeElement) {
        const classList = [...document.activeElement.classList];
        if (classList.includes("xterm-helper-textarea")) {
          (document.activeElement as HTMLElement).blur();
        }
      }

      showNetworkInfo = false;
    });
  });

  /** Returns the mouse position in infinite grid coordinates, offset transformations and zoom. */
  function normalizePosition(event: MouseEvent): [number, number] {
    const [ox, oy] = getConstantOffset();
    return [
      Math.round(center[0] + event.pageX / zoom - ox),
      Math.round(center[1] + event.pageY / zoom - oy),
    ];
  }

  function isCoarsePointer() {
    return (
      typeof matchMedia === "function" &&
      matchMedia("(hover: none), (pointer: coarse)").matches
    );
  }

  function clampInt(value: number, min: number, max: number) {
    if (!Number.isFinite(value)) return min;
    return Math.min(max, Math.max(min, Math.floor(value)));
  }

  let encrypt: Encrypt;
  let srocket: Srocket<WsServer, WsClient> | null = null;

  let connected = false;
  let exitReason: string | null = null;

  /** Bound "write" method for each terminal. */
  const writers: Record<number, (data: string) => void> = {};
  const termWrappers: Record<number, HTMLDivElement> = {};
  const termElements: Record<number, HTMLDivElement> = {};
  const chunknums: Record<number, number> = {};
  const locks: Record<number, any> = {};
  let userId = 0;
  let users: [number, WsUser][] = [];
  let shells: [number, WsWinsize][] = [];
  let subscriptions = new Set<number>();

  // May be undefined before `users` is first populated.
  $: hasWriteAccess = users.find(([uid]) => uid === userId)?.[1]?.canWrite;

  let moving = -1; // Terminal ID that is being dragged.
  let movingPointerId: number | null = null; // Track pointer ID during drag gesture.
  let movingOrigin = [0, 0]; // Coordinates of mouse at origin when drag started.
  let movingSize: WsWinsize; // New [x, y] position of the dragged terminal.
  let movingIsDone = false; // Moving finished but hasn't been acknowledged.

  // Soft alignment snapping — shared with board items via the <Board> guides.
  const SNAP_PX = 8; // screen-space pull distance (÷ zoom → world units)
  let termGuidesV: number[] = []; // active guide lines while dragging a terminal
  let termGuidesH: number[] = [];
  let edgeSnapPreview: ViewRect | null = null;
  let pendingEdgeSnap: { id: number; action: SnapAction } | null = null;
  let snapRestore: Record<number, WsWinsize> = {};
  let snapHistory: Record<number, { action: SnapAction; rect: ViewRect }> = {};
  let snapPadFor: number | null = null;
  let layoutModeId: number | null = null;
  let layoutModeTimer: ReturnType<typeof setTimeout> | null = null;

  let resizing = -1; // Terminal ID that is being resized.
  let resizingPointerId: number | null = null; // Track pointer ID during resize gesture.
  let resizingOrigin = [0, 0]; // Coordinates of top-left origin when resize started.
  let resizingCell = [0, 0]; // Pixel dimensions of a single terminal cell.
  let resizingSize: WsWinsize; // Last resize message sent.

  let chatMessages: ChatMessage[] = [];
  let newMessages = false;
  let initialShellsReceived = false;
  let didInitialFit = false; // auto-fit the view once on first load
  // Set when we joined a fresh empty session before write-perms were known, so
  // the auto-create can fire once canWrite arrives (handleCreate needs canEdit).
  let autoCreatePending = false;
  let pendingPortraitCreates: { x: number; y: number; expiresAt: number }[] = [];

  // ── maw share workboard extensions ──
  let boardItems: BoardItem[] = [];
  // Live screen-share frames as object URLs, keyed by board item id.
  let streamSrcs: Record<string, string> = {};
  let micRecording = false;
  // WebRTC P2P mesh for voice/video (replaces WS voice relay for low latency).
  let rtcMesh: RtcMesh | null = null;
  let micStream: MediaStream | null = null;
  let cameraStream: MediaStream | null = null;
  let cameraActive = false;
  // Remote audio elements keyed by peer UID.
  let remoteAudios: Record<number, HTMLAudioElement> = {};
  let remoteVideos: Record<number, MediaStream> = {}; // live remote camera streams
  let stream: StreamController | null = null;
  let streamActive = false;
  let myStreamId: string | null = null;

  /** Insert or replace a board item by id. */
  function upsertBoardItem(item: BoardItem) {
    const idx = boardItems.findIndex((it) => it.id === item.id);
    if (idx === -1) boardItems = [...boardItems, item];
    else {
      boardItems[idx] = item;
      boardItems = boardItems;
    }
  }

  /** Remove a board item and release any live stream frame URL. */
  function removeBoardItem(id: string) {
    boardItems = boardItems.filter((it) => it.id !== id);
    if (streamSrcs[id]) {
      URL.revokeObjectURL(streamSrcs[id]);
      delete streamSrcs[id];
      streamSrcs = streamSrcs;
    }
  }

  /** Update the live frame shown for a stream tile. */
  function setStreamFrame(id: string, bytes: Uint8Array) {
    const url = URL.createObjectURL(new Blob([bytes], { type: "image/jpeg" }));
    if (streamSrcs[id]) URL.revokeObjectURL(streamSrcs[id]);
    streamSrcs = { ...streamSrcs, [id]: url };
  }

  /** A world-space position near the current view center for new board items. */
  function nextBoardPos(): [number, number] {
    const ox = ((boardItems.length * 28) % 200) - 100;
    const oy = ((boardItems.length * 22) % 160) - 80;
    return [Math.round(center[0] + ox), Math.round(center[1] + oy)];
  }

  let serverLatencies: number[] = [];
  let shellLatencies: number[] = [];

  onMount(async () => {
    // The page hash sets the end-to-end encryption key.
    const key = window.location.hash?.slice(1).split(",")[0] ?? "";
    const writePassword = window.location.hash?.slice(1).split(",")[1] ?? null;

    encrypt = await Encrypt.new(key);
    const encryptedZeros = await encrypt.zeros();

    const writeEncryptedZeros = writePassword
      ? await (await Encrypt.new(writePassword)).zeros()
      : null;

    srocket = new Srocket<WsServer, WsClient>(`/api/s/${id}`, {
      onMessage(message) {
        if (message.hello) {
          userId = message.hello[0];
          dispatch("receiveName", message.hello[1]);
          makeToast({
            kind: "success",
            message: `Connected to the server.`,
          });
          exitReason = null;
          // Initialize WebRTC mesh now that we know our UID.
          rtcMesh?.dispose();
          rtcMesh = new RtcMesh(
            userId,
            (target, payload) => srocket?.send({ signal: [target, payload] }),
            (uid, track, streams) => {
              if (track.kind === "audio") {
                const audio = new Audio();
                audio.srcObject = new MediaStream([track]);
                audio.play().catch(() => {});
                remoteAudios[uid] = audio;
              } else if (track.kind === "video") {
                // Remote camera — render the live MediaStream directly into a
                // <video> element (smooth WebRTC video, no JPEG snapshotting).
                const stream = streams[0] ?? new MediaStream([track]);
                remoteVideos = { ...remoteVideos, [uid]: stream };
                track.addEventListener("ended", () => {
                  const { [uid]: _gone, ...rest } = remoteVideos;
                  remoteVideos = rest;
                });
              }
            },
          );
        } else if (message.invalidAuth) {
          exitReason =
            "The URL is not correct, invalid end-to-end encryption key.";
          srocket?.dispose();
        } else if (message.chunks) {
          let [id, seqnum, chunks] = message.chunks;
          locks[id](async () => {
            await tick();
            chunknums[id] += chunks.length;
            for (const data of chunks) {
              const buf = await encrypt.segment(
                0x100000000n | BigInt(id),
                BigInt(seqnum),
                data,
              );
              seqnum += data.length;
              writers[id](new TextDecoder().decode(buf));
            }
          });
        } else if (message.users) {
          users = message.users;
          for (const [uid] of users) rtcMesh?.addPeer(uid);
        } else if (message.userDiff) {
          const [id, update] = message.userDiff;
          users = users.filter(([uid]) => uid !== id);
          if (update !== null) {
            users = [...users, [id, update]];
            rtcMesh?.addPeer(id);
          } else {
            rtcMesh?.removePeer(id);
            remoteAudios[id]?.pause();
            delete remoteAudios[id];
            const { [id]: _goneVideo, ...restVideos } = remoteVideos;
            remoteVideos = restVideos;
          }
        } else if (message.shells) {
          const previousShellIds = new Set(shells.map(([id]) => id));
          shells = message.shells;
          if (movingIsDone) {
            moving = -1;
          }
          // Auto-create one terminal when joining a fresh empty session. If
          // write-perms aren't known yet (undefined), defer — handleCreate needs
          // canEdit===true, and a premature call both no-ops and toasts.
          if (!initialShellsReceived && message.shells.length === 0) {
            if (hasWriteAccess === true) handleCreate();
            else if (hasWriteAccess === undefined) autoCreatePending = true;
          }
          initialShellsReceived = true;
          for (const [id] of message.shells) {
            if (!subscriptions.has(id)) {
              chunknums[id] ??= 0;
              locks[id] ??= createLock();
              subscriptions.add(id);
              srocket?.send({ subscribe: [id, chunknums[id]] });
            }
          }
          applyPendingPortraitDefaults(message.shells, previousShellIds);
        } else if (message.hear) {
          const [uid, name, msg] = message.hear;
          chatMessages.push({ uid, name, msg, sentAt: new Date() });
          chatMessages = chatMessages;
          if (!showChat) newMessages = true;
        } else if (message.voiceData) {
          playVoice(message.voiceData[1]);
        } else if (message.streamFrame) {
          const [, streamId, bytes] = message.streamFrame;
          setStreamFrame(streamId, bytes);
        } else if (message.board) {
          boardItems = message.board;
        } else if (message.boardPut) {
          upsertBoardItem(message.boardPut);
        } else if (message.boardMove) {
          const [id, x, y] = message.boardMove;
          const item = boardItems.find((it) => it.id === id);
          if (item) upsertBoardItem({ ...item, x, y });
        } else if (message.boardDelete) {
          removeBoardItem(message.boardDelete);
        } else if (message.signal) {
          // Signal format: [from_uid, payload] (targeted) or
          // [from_uid, to_uid, payload] (broadcast — client filters).
          const sig = message.signal as number[] & string[];
          if (sig.length === 3) {
            const [from, to, payload] = sig as unknown as [number, number, string];
            if (to === userId) rtcMesh?.handleSignal(from, payload).catch(() => {});
          } else {
            const [from, payload] = sig as unknown as [number, string];
            rtcMesh?.handleSignal(from, payload).catch(() => {});
          }
        } else if (message.shellLatency !== undefined) {
          const shellLatency = Number(message.shellLatency);
          shellLatencies = [...shellLatencies, shellLatency].slice(-10);
        } else if (message.pong !== undefined) {
          const serverLatency = Date.now() - Number(message.pong);
          serverLatencies = [...serverLatencies, serverLatency].slice(-10);
        } else if (message.error) {
          console.warn("Server error: " + message.error);
        }
      },

      onConnect() {
        srocket?.send({ authenticate: [encryptedZeros, writeEncryptedZeros] });
        if ($settings.name) {
          srocket?.send({ setName: $settings.name });
        }
        connected = true;
      },

      onDisconnect() {
        connected = false;
        subscriptions.clear();
        users = [];
        serverLatencies = [];
        shellLatencies = [];
        rtcMesh?.dispose();
        rtcMesh = null;
        for (const audio of Object.values(remoteAudios)) audio.pause();
        remoteAudios = {};
      },

      onClose(event) {
        if (event.code === 4404) {
          exitReason = "Failed to connect: " + event.reason;
        } else if (event.code === 4500) {
          exitReason = "Internal server error: " + event.reason;
        }
      },
    });
  });

  onDestroy(() => srocket?.dispose());

  // Send periodic ping messages for latency estimation.
  onMount(() => {
    const pingIntervalId = window.setInterval(() => {
      if (srocket?.connected) {
        srocket.send({ ping: BigInt(Date.now()) });
      }
    }, 2000);
    return () => window.clearInterval(pingIntervalId);
  });

  function integerMedian(values: number[]) {
    if (values.length === 0) {
      return null;
    }
    const sorted = values.toSorted();
    const mid = Math.floor(sorted.length / 2);
    return sorted.length % 2 !== 0
      ? sorted[mid]
      : Math.round((sorted[mid - 1] + sorted[mid]) / 2);
  }

  $: if ($settings.name) {
    srocket?.send({ setName: $settings.name });
  }

  let counter = 0n;

  function rememberPendingPortraitCreate(x: number, y: number) {
    pendingPortraitCreates = [
      ...pendingPortraitCreates,
      { x, y, expiresAt: Date.now() + PENDING_CREATE_TTL_MS },
    ];
  }

  function applyPendingPortraitDefaults(
    nextShells: [number, WsWinsize][],
    previousShellIds: Set<number>,
  ) {
    if (!canEdit || pendingPortraitCreates.length === 0) return;

    const now = Date.now();
    const pending = pendingPortraitCreates.filter((p) => p.expiresAt > now);
    const created = nextShells.filter(([sid]) => !previousShellIds.has(sid));
    if (created.length === 0) {
      pendingPortraitCreates = pending;
      return;
    }

    for (const [id, ws] of created) {
      if (pending.length === 0) break;
      const matchIndex = pending.findIndex(
        (p) => Math.abs(p.x - ws.x) <= 12 && Math.abs(p.y - ws.y) <= 12,
      );
      if (matchIndex === -1) continue;
      pending.splice(matchIndex, 1);
      const cols = clampInt(
        DEFAULT_PORTRAIT_COLS,
        TERM_MIN_COLS,
        TERM_MAX_COLS,
      );
      const rows = clampInt(
        DEFAULT_PORTRAIT_ROWS,
        TERM_MIN_ROWS,
        TERM_MAX_ROWS,
      );
      if (ws.cols !== cols || ws.rows !== rows) {
        srocket?.send({ move: [id, { ...ws, cols, rows }] });
      }
    }

    pendingPortraitCreates = pending;
  }

  async function handleCreate() {
    if (!canEdit) {
      makeToast({
        kind: "info",
        message: lockedForMe
          ? `Board is locked by ${boardLock?.ownerName ?? "someone"} — read-only.`
          : "You are in read-only mode and cannot create new terminals.",
      });
      return;
    }
    if (shells.length >= 14) {
      makeToast({
        kind: "error",
        message: "You can only create up to 14 terminals.",
      });
      return;
    }
    const existing = shells.map(([id, winsize]) => ({
      x: winsize.x,
      y: winsize.y,
      width: termWrappers[id].clientWidth,
      height: termWrappers[id].clientHeight,
    }));
    const { x, y } = arrangeNewTerminal(existing);
    rememberPendingPortraitCreate(x, y);
    srocket?.send({ create: [x, y] });
    touchZoom.moveTo([x, y], INITIAL_ZOOM);
  }

  async function handleInput(id: number, data: Uint8Array) {
    if (!canEdit) return; // soft lock: swallow keystrokes for locked-out users
    if (counter === 0n) {
      // On the first call, initialize the counter to a random 64-bit integer.
      const array = new Uint8Array(8);
      crypto.getRandomValues(array);
      counter = new DataView(array.buffer).getBigUint64(0);
    }
    const offset = counter;
    counter += BigInt(data.length); // Must increment before the `await`.
    const encrypted = await encrypt.segment(0x200000000n, offset, data);
    srocket?.send({ data: [id, encrypted, offset] });
  }

  // ── Broadcast input (Bo 2026-06-13) ───────────────────────────────────────
  // When on, a keystroke in any terminal is mirrored to every terminal — handy
  // for running the same command across many SSH sessions. Routes through the
  // proven per-call handleInput (global counter stays correct), so no new crypto.
  let broadcastMode = false;
  function routeInput(originId: number, data: Uint8Array) {
    if (hasWriteAccess === false || lockedForMe) return;
    if (broadcastMode && shells.length > 1) {
      for (const [sid] of shells) handleInput(sid, data);
    } else {
      handleInput(originId, data);
    }
  }

  // Paste a saved snippet into the focused terminal (honours broadcast + lock).
  function pasteSnippet(text: string) {
    if (hasWriteAccess === false || lockedForMe) {
      makeToast({ kind: "info", message: "Read-only mode — can't paste." });
      return;
    }
    // Clicking the snippet panel blurs the terminal, so fall back to the
    // last-focused shell (if it still exists) instead of the live focus.
    const live = focused[0];
    const target =
      live !== undefined
        ? live
        : shells.some(([sid]) => sid === lastFocused)
          ? lastFocused
          : undefined;
    if (target === undefined) {
      makeToast({
        kind: "info",
        message: "Click a terminal first, then paste.",
      });
      return;
    }
    routeInput(target, new TextEncoder().encode(text));
  }

  function handleNumpadPress(key: string) {
    const live = focused[0];
    const target =
      live !== undefined
        ? live
        : shells.some(([sid]) => sid === lastFocused)
          ? lastFocused
          : undefined;
    if (target === undefined) {
      makeToast({
        kind: "info",
        message: "Tap a terminal first.",
      });
      return;
    }

    const text =
      key === "Enter"
        ? "\r"
        : key === "Backspace"
          ? "\x7f"
          : key === "ArrowUp"
            ? "\x1b[A"
            : key === "ArrowDown"
              ? "\x1b[B"
              : key === "ArrowRight"
                ? "\x1b[C"
                : key === "ArrowLeft"
                  ? "\x1b[D"
                  : key;
    handleInput(target, new TextEncoder().encode(text));
  }

  function handleNumpadSnap(action: string) {
    if (!canEdit) {
      makeToast({ kind: "info", message: "Read-only mode." });
      return;
    }
    const id = shortcutTargetId();
    if (id === null) {
      makeToast({ kind: "info", message: "Tap a terminal first." });
      return;
    }
    handleSnapButton(id, action);
  }

  // ── Terminal labels (Bo 2026-06-13) ───────────────────────────────────────
  // Per-shell names synced to all peers via board items (kind:"label",
  // id "__label_<sid>"). Makes a multi-server board legible — and broadcast safe.
  $: labels = (() => {
    const m: Record<number, string> = {};
    for (const it of boardItems) {
      if (it.kind === "label" && it.id.startsWith("__label_")) {
        const sid = Number(it.id.slice("__label_".length));
        if (!Number.isNaN(sid)) m[sid] = it.dataUrl;
      }
    }
    return m;
  })();

  function handleRename(id: number, text: string) {
    if (!canEdit) return;
    const itemId = `__label_${id}`;
    if (!text) {
      removeBoardItem(itemId);
      srocket?.send({ boardDelete: itemId });
      return;
    }
    const item: BoardItem = {
      id: itemId,
      kind: "label",
      x: 0,
      y: 0,
      w: 0,
      h: 0,
      dataUrl: text,
    };
    upsertBoardItem(item);
    srocket?.send({ boardPut: item });
  }

  function handleStartMove(id: number, ws: WsWinsize, event: any) {
    if (!canEdit) return;
    const [x, y] = normalizePosition(event);
    moving = id;
    movingPointerId = event.pointerId ?? null;
    movingOrigin = [x - ws.x, y - ws.y];
    movingSize = ws;
    movingIsDone = false;
  }

  // Stupid hack to preserve input focus when terminals are reordered.
  // See: https://github.com/sveltejs/svelte/issues/3973
  let activeElement: Element | null = null;

  beforeUpdate(() => {
    activeElement = document.activeElement;
  });

  afterUpdate(() => {
    if (activeElement instanceof HTMLElement) activeElement.focus();
  });

  // Global mouse handler logic follows, attached to the window element for smoothness.
  onMount(() => {
    // 50 milliseconds between successive terminal move updates.
    const sendMove = throttle((message: WsClient) => {
      srocket?.send(message);
    }, 50);

    // 80 milliseconds between successive cursor updates.
    const sendCursor = throttle((message: WsClient) => {
      srocket?.send(message);
    }, 80);

    function handleMouse(event: MouseEvent) {
      if (moving !== -1 && !movingIsDone) {
        if (event instanceof PointerEvent && movingPointerId !== null && event.pointerId !== movingPointerId) {
          return;
        }
        const [x, y] = normalizePosition(event);
        let nx = Math.round(x - movingOrigin[0]);
        let ny = Math.round(y - movingOrigin[1]);
        // Soft-snap the window to align with other windows + board items.
        const el = termWrappers[moving];
        if (el && snapTargets.length) {
          const others = snapTargets.filter((t) => t.id !== `t${moving}`);
          const r = computeSnap(
            nx,
            ny,
            el.offsetWidth, // layout px = world px (transform scale is paint-only)
            el.offsetHeight,
            others,
            SNAP_PX / zoom, // screen px → world px for the pull threshold
          );
          nx = r.x;
          ny = r.y;
          termGuidesV = r.guidesV;
          termGuidesH = r.guidesH;
        }
        const edgeAction = detectEdgeSnapAction(
          event.clientX,
          event.clientY,
          { w: window.innerWidth, h: window.innerHeight },
          { coarse: isCoarsePointer() },
        );
        if (edgeAction) {
          const view = visibleWorldRect();
          const preview = computeSnapTarget(edgeAction, view, {
            x: nx,
            y: ny,
            w:
              el && el.offsetWidth > 0
                ? el.offsetWidth
                : movingSize.cols * 9.6 + 36,
            h:
              el && el.offsetHeight > 0
                ? el.offsetHeight
                : movingSize.rows * 19 + 60,
          });
          edgeSnapPreview = applySnapGap(
            preview,
            $settings.snapGap,
            snapSharedEdges(edgeAction, view),
          );
          pendingEdgeSnap = { id: moving, action: edgeAction };
          termGuidesV = [];
          termGuidesH = [];
        } else {
          edgeSnapPreview = null;
          pendingEdgeSnap = null;
        }
        movingSize = { ...movingSize, x: nx, y: ny };
        sendMove({ move: [moving, movingSize] });
      }

      if (resizing !== -1) {
        if (event instanceof PointerEvent && resizingPointerId !== null && event.pointerId !== resizingPointerId) {
          return;
        }
        const cols = Math.max(
          Math.floor((event.pageX - resizingOrigin[0]) / resizingCell[0]),
          TERM_MIN_COLS, // Minimum number of columns.
        );
        const rows = Math.max(
          Math.floor((event.pageY - resizingOrigin[1]) / resizingCell[1]),
          TERM_MIN_ROWS, // Minimum number of rows.
        );
        if (rows !== resizingSize.rows || cols !== resizingSize.cols) {
          resizingSize = { ...resizingSize, rows, cols };
          srocket?.send({ move: [resizing, resizingSize] });
        }
      }

      sendCursor({ setCursor: normalizePosition(event) });
    }

    function handleMouseEnd(event: MouseEvent) {
      if (moving !== -1) {
        if (event instanceof PointerEvent && movingPointerId !== null && event.pointerId !== movingPointerId) {
          return;
        }
        movingIsDone = true;
        sendMove.cancel();
        let edgeSnap =
          pendingEdgeSnap?.id === moving ? pendingEdgeSnap : null;
        const mayEdgeSnap = event.type === "pointerup" || event.type === "mouseup";
        if (!edgeSnap && mayEdgeSnap) {
          const edgeAction = detectEdgeSnapAction(
            event.clientX,
            event.clientY,
            { w: window.innerWidth, h: window.innerHeight },
            { coarse: isCoarsePointer() },
          );
          if (edgeAction) edgeSnap = { id: moving, action: edgeAction };
        }
        if (edgeSnap && mayEdgeSnap) {
          void applySnap(moving, edgeSnap.action, false, { cycle: false });
        } else {
          srocket?.send({ move: [moving, movingSize] });
        }
        edgeSnapPreview = null;
        pendingEdgeSnap = null;
        termGuidesV = [];
        termGuidesH = [];
        movingPointerId = null;
      }

      if (resizing !== -1) {
        if (event instanceof PointerEvent && resizingPointerId !== null && event.pointerId !== resizingPointerId) {
          return;
        }
        resizing = -1;
        resizingPointerId = null;
      }

      if (event.type === "pointerleave") {
        sendCursor.cancel();
        srocket?.send({ setCursor: null });
      }
    }

    // Pointer events cover mouse, touch, and pen — so terminal move/resize
    // works on mobile (touch) as well as desktop.
    window.addEventListener("pointermove", handleMouse);
    window.addEventListener("pointerup", handleMouseEnd);
    window.addEventListener("pointercancel", handleMouseEnd);
    document.body.addEventListener("pointerleave", handleMouseEnd);
    return () => {
      window.removeEventListener("pointermove", handleMouse);
      window.removeEventListener("pointerup", handleMouseEnd);
      window.removeEventListener("pointercancel", handleMouseEnd);
      document.body.removeEventListener("pointerleave", handleMouseEnd);
      edgeSnapPreview = null;
      pendingEdgeSnap = null;
    };
  });

  onMount(() => {
    function handleLayoutKey(event: KeyboardEvent) {
      if (layoutModeId === null) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        clearLayoutMode();
        return;
      }

      const action = layoutKeyAction(event);
      if (action === null) return;
      event.preventDefault();
      event.stopPropagation();
      refreshLayoutModeTimeout();

      const id = shells.some(([sid]) => sid === layoutModeId)
        ? layoutModeId
        : activeTerminalId();
      if (id === null) {
        clearLayoutMode();
        return;
      }
      layoutModeId = id;
      if (action === "restore") restoreSnap(id);
      else void applySnap(id, action);
    }

    window.addEventListener("keydown", handleLayoutKey, true);
    return () => {
      window.removeEventListener("keydown", handleLayoutKey, true);
      if (layoutModeTimer) {
        clearTimeout(layoutModeTimer);
        layoutModeTimer = null;
      }
    };
  });

  onMount(() => {
    function handleGlobalSnapShortcut(event: KeyboardEvent) {
      if (event.key === "Escape" && snapPadFor !== null) {
        snapPadFor = null;
        return;
      }

      const action = snapShortcutAction(event);
      if (action === null) return;

      event.preventDefault();
      event.stopPropagation();

      const id = shortcutTargetId();
      if (id === null) {
        makeToast({ kind: "info", message: "Tap a terminal first." });
        return;
      }

      snapPadFor = null;
      handleShortcutSnap(id, action);
    }

    window.addEventListener("keydown", handleGlobalSnapShortcut, true);
    return () =>
      window.removeEventListener("keydown", handleGlobalSnapShortcut, true);
  });

  let focused: number[] = [];
  let lastFocused = -1; // most-recent focused shell; survives blur for snippet paste
  $: setFocus(focused);

  // Wait a small amount of time, since blur events happen before focus events.
  const setFocus = debounce((focused: number[]) => {
    srocket?.send({ setFocus: focused[0] ?? null });
  }, 20);

  // ── maw share workboard handlers ──

  // Mic toggle: add/remove audio track from WebRTC mesh (P2P, low latency).
  // Falls back to WS Voice relay if no peers connected yet.
  async function handleMicDown() {
    if (micRecording && micStream) {
      // Stop mic — remove track from mesh, stop stream.
      for (const track of micStream.getAudioTracks()) {
        rtcMesh?.removeTrack(track);
        track.stop();
      }
      micStream = null;
      micRecording = false;
      return;
    }
    try {
      micStream = await navigator.mediaDevices.getUserMedia({ audio: true });
      for (const track of micStream.getAudioTracks()) {
        rtcMesh?.addTrack(track);
      }
      micRecording = true;
    } catch {
      makeToast({ kind: "error", message: "Microphone blocked." });
    }
  }

  // Add an image to the board (file picker, paste, or drag-and-drop).
  // Local preview uses the original blob (sharp); peers get the encoded share.
  function addImage(file: File) {
    if (!canEdit) return;
    const id = crypto.randomUUID();
    const localUrl = URL.createObjectURL(file);
    streamSrcs = { ...streamSrcs, [id]: localUrl };

    readImageFile(file, (payload) => {
      const [x, y] = nextBoardPos();
      const item: BoardItem = {
        id,
        kind: "image",
        x,
        y,
        w: payload.tileW,
        h: payload.tileH,
        dataUrl: payload.dataUrl,
      };
      upsertBoardItem(item);
      srocket?.send({ boardPut: item });
    });
  }

  // Add an editable sticky note to the board.
  function addNote() {
    if (!canEdit) return;
    const [x, y] = nextBoardPos();
    const item: BoardItem = {
      id: crypto.randomUUID(),
      kind: "note",
      x,
      y,
      w: 220,
      h: 160,
      dataUrl: "", // note text lives in dataUrl
    };
    upsertBoardItem(item);
    srocket?.send({ boardPut: item });
  }

  // Shared markdown document — a singleton board item synced to all peers
  // (hidden from the board canvas; shown only in the Document panel).
  const DOC_ID = "__shared_doc__";
  $: docText = boardItems.find((it) => it.id === DOC_ID)?.dataUrl ?? "";

  function handleDocEdit(text: string) {
    if (!canEdit) return;
    const item: BoardItem = {
      id: DOC_ID,
      kind: "doc",
      x: 0,
      y: 0,
      w: 0,
      h: 0,
      dataUrl: text,
    };
    upsertBoardItem(item);
    srocket?.send({ boardPut: item });
  }

  // ── Read-only file viewer (Bo 2026-06-13) ─────────────────────────────────
  // Clicking a file in the explorer loads its content into a shared singleton
  // board item, so peers see what's being viewed ("เพื่อนอยากเห็นว่าเขียนอะไร").
  // Content comes from the sandboxed /api/file route (text-only, no secrets).
  const FILE_VIEW_ID = "__file_view__";
  $: fileViewText =
    boardItems.find((it) => it.id === FILE_VIEW_ID)?.dataUrl ?? "";
  let showFileView = false;
  let lastFileView = "";
  // Auto-open for everyone when the shared file changes, without fighting a
  // manual close (only re-open when a *different* file is opened).
  $: if (fileViewText && fileViewText !== lastFileView) {
    lastFileView = fileViewText;
    showFileView = true;
  }

  async function openFile(path: string) {
    // Opening a file writes the shared FILE_VIEW_ID board item (boardPut below),
    // so it is a board mutation and must respect the same lock as every other
    // mutation. The soft-lock is client-side and the server only enforces
    // canWrite, so without this gate a write-capable but locked-out user could
    // change the shared file view for everyone.
    if (!canEdit) {
      makeToast({
        kind: "error",
        message: lockedForMe ? "Board is locked" : "Read-only access",
      });
      return;
    }
    try {
      const res = await fetch(`/api/file?path=${encodeURIComponent(path)}`);
      if (!res.ok) {
        makeToast({
          kind: "error",
          message: res.status === 403 ? "Restricted file" : "Can't open file",
        });
        return;
      }
      const { content } = await res.json();
      const item: BoardItem = {
        id: FILE_VIEW_ID,
        kind: "doc",
        x: 0,
        y: 0,
        w: 0,
        h: 0,
        dataUrl: `📄 ${path}\n\n${content}`,
      };
      upsertBoardItem(item);
      srocket?.send({ boardPut: item });
      showFileView = true;
    } catch {
      makeToast({ kind: "error", message: "Can't open file" });
    }
  }

  // ── Soft board lock (Bo 2026-06-13) ───────────────────────────────────────
  // A singleton board item (rides the existing board sync — no server change)
  // that flips everyone except the locker into read-only. "Soft": it gates the
  // client UI to stop accidental edits, not a cryptographic write-lock.
  const LOCK_ID = "__board_lock__";
  $: lockItem = boardItems.find((it) => it.id === LOCK_ID);
  $: boardLock = (() => {
    try {
      return lockItem?.dataUrl ? JSON.parse(lockItem.dataUrl) : null;
    } catch {
      return null;
    }
  })();
  $: boardLocked = !!boardLock?.locked;
  $: lockedForMe = boardLocked && boardLock.ownerId !== userId;
  // Combined edit permission: have CONFIRMED server write access AND not locked
  // out. Treating `undefined` (permission not yet known) as editable let the UI
  // make phantom local mutations before perms loaded, which the server then
  // rejected -> board desync. Require an explicit `true`.
  $: canEdit = hasWriteAccess === true && !lockedForMe;

  // Auto-fit the view once on first load so every device opens centered with
  // everything visible. Must wait until terminals have actually rendered — a
  // fixed delay was unreliable (firing before offsetWidth was set made
  // fitToContent see no measurable rects and fall back to the world origin,
  // leaving content stuck off-screen top-left). Retry until measurable.
  $: if (!didInitialFit && initialShellsReceived && shells.length > 0) {
    didInitialFit = true;
    // Small delay so a terminal renders first (fitToContent measures cell size
    // from it); the box math falls back to an estimate if not, so this no longer
    // has to wait for a precise layout.
    setTimeout(() => fitToContent(), 400);
  }

  // Fire the deferred auto-create once write-perms arrive (still empty board),
  // or drop it if perms came back read-only / someone else already created one.
  $: if (autoCreatePending) {
    if (hasWriteAccess === false || shells.length > 0) {
      autoCreatePending = false;
    } else if (canEdit && shells.length === 0) {
      autoCreatePending = false;
      handleCreate();
    }
  }

  // Apply the user's panel/header color (Settings) as a CSS variable that
  // `.panel` reads; empty value falls back to the built-in default.
  $: if (typeof document !== "undefined") {
    if ($settings.panelBackground) {
      document.documentElement.style.setProperty(
        "--panel-bg",
        $settings.panelBackground,
      );
    } else {
      document.documentElement.style.removeProperty("--panel-bg");
    }
  }

  // World-space rects of everything draggable (terminals + board items) so a
  // drag can soft-snap to align with its neighbours. Terminal pixel size comes
  // from the live DOM (÷ zoom → world units); recomputed when shells/items move.
  $: snapTargets = (() => {
    const rects: SnapRect[] = [];
    for (const [id, ws] of shells) {
      const el = termWrappers[id];
      // offsetWidth/Height are layout (world) px — the canvas `transform: scale`
      // is paint-only and doesn't change them, so NO ÷ zoom here. (Also keeps
      // this reactive off the zoom dep → no layout reads during pan/zoom.)
      if (!el || el.offsetWidth <= 0) continue;
      rects.push({
        id: `t${id}`,
        left: ws.x,
        top: ws.y,
        width: el.offsetWidth,
        height: el.offsetHeight,
      });
    }
    for (const it of boardItems) {
      if (it.kind === "doc" || it.kind === "lock" || it.kind === "label") continue;
      // Skip degenerate rects (e.g. videos created with h=0) — a zero-height
      // rect would place bogus middle/bottom guides.
      if (!it.w || !it.h) continue;
      rects.push({ id: it.id, left: it.x, top: it.y, width: it.w, height: it.h });
    }
    return rects;
  })();

  // Paint the board background onto <html> + <body> too. <main> only spans ~58px
  // (toolbar strip); html is transparent and body is the built-in #0e0e10, so the
  // top safe-area band rendered white through the address bar. Mirroring the
  // board color everywhere keeps the whole page one seamless colour. (Bo 2026-06-13)
  $: if (typeof document !== "undefined") {
    document.documentElement.style.backgroundColor = $settings.background;
    document.body.style.backgroundColor = $settings.background;
  }

  function toggleLock() {
    if (hasWriteAccess === false) {
      makeToast({
        kind: "info",
        message: "Read-only viewers can't lock the board.",
      });
      return;
    }
    // Soft lock is cooperative: anyone with write access can lock OR unlock, so
    // the board never gets stuck if the locker leaves. Ownership is just a label.
    const nowLocked = !boardLocked;
    const item: BoardItem = {
      id: LOCK_ID,
      kind: "lock",
      x: 0,
      y: 0,
      w: 0,
      h: 0,
      dataUrl: JSON.stringify(
        nowLocked
          ? {
              locked: true,
              ownerId: userId,
              ownerName: $settings.name || "someone",
            }
          : { locked: false },
      ),
    };
    upsertBoardItem(item);
    srocket?.send({ boardPut: item });
    makeToast({
      kind: "success",
      message: nowLocked
        ? "🔒 Board locked — everyone else is read-only"
        : "🔓 Board unlocked",
    });
  }

  // Persist a note's edited text to peers.
  function handleNoteEdit(id: string, text: string) {
    if (!canEdit) return;
    const item = boardItems.find((it) => it.id === id);
    if (!item) return;
    const updated = { ...item, dataUrl: text };
    upsertBoardItem(updated);
    srocket?.send({ boardPut: updated });
  }

  function handleImage() {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/*";
    input.multiple = true;
    input.onchange = () => {
      for (const file of input.files ?? []) addImage(file);
    };
    input.click();
  }

  // Above this, the whole file won't fit in a WS message — show it locally but
  // don't broadcast the data URL to peers.
  const VIDEO_SHARE_CAP = 20 * 1024 * 1024; // 20 MB

  // Add a video clip to the board. Local preview works at ANY size via an
  // object URL; the file is shared with peers (as a data URL) only when small
  // enough to fit over the WS. A download button on the tile lets anyone save it.
  function addVideoFile(file: File) {
    if (!canEdit) return;
    if (!file.type.startsWith("video/")) return;

    const [x, y] = nextBoardPos();
    const id = crypto.randomUUID();
    const item: BoardItem = {
      id,
      kind: "video",
      x,
      y,
      w: 480,
      h: 0,
      // Instant local preview at any size (blob URL stays on this machine).
      dataUrl: URL.createObjectURL(file),
    };
    upsertBoardItem(item);

    if (file.size <= VIDEO_SHARE_CAP) {
      // Small enough to share: re-send as a data URL so peers can see + save it.
      const reader = new FileReader();
      reader.onload = () => {
        const shared = { ...item, dataUrl: String(reader.result) };
        srocket?.send({ boardPut: shared });
      };
      reader.readAsDataURL(file);
    } else {
      makeToast({
        kind: "info",
        message: "Video shown here — too large to share with others.",
      });
    }
  }

  function handleVideoPick() {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "video/*";
    input.onchange = () => {
      for (const file of input.files ?? []) addVideoFile(file);
    };
    input.click();
  }

  onMount(() => {
    const onPaste = (event: ClipboardEvent) => {
      for (const item of event.clipboardData?.items ?? []) {
        if (item.type.startsWith("image/")) {
          const file = item.getAsFile();
          if (file) addImage(file);
        }
      }
    };
    const onDragOver = (event: DragEvent) => event.preventDefault();
    const onDrop = (event: DragEvent) => {
      event.preventDefault();
      for (const file of event.dataTransfer?.files ?? []) {
        if (file.type.startsWith("video/")) addVideoFile(file);
        else addImage(file);
      }
    };
    // Re-fit on rotate (the main "responsive" trigger on tablets/phones).
    // Debounced + skipped while the user is mid-drag/resize so we don't fight
    // a manual interaction.
    let fitTimer: ReturnType<typeof setTimeout> | null = null;
    const onOrient = () => {
      if (fitTimer) clearTimeout(fitTimer);
      fitTimer = setTimeout(() => {
        if (moving === -1 && resizing === -1) fitToContent();
      }, 300);
    };
    window.addEventListener("paste", onPaste);
    window.addEventListener("dragover", onDragOver);
    window.addEventListener("drop", onDrop);
    window.addEventListener("orientationchange", onOrient);
    return () => {
      window.removeEventListener("paste", onPaste);
      window.removeEventListener("dragover", onDragOver);
      window.removeEventListener("drop", onDrop);
      window.removeEventListener("orientationchange", onOrient);
      if (fitTimer) clearTimeout(fitTimer);
    };
  });

  // Screen share: getDisplayMedia → send video track via WebRTC to peers +
  // place a board tile (WS StreamFrame fallback for board preview at ~3fps).
  let displayStream: MediaStream | null = null;

  async function handleStream() {
    if (!canEdit) return;
    if (stream) {
      stopStream();
      return;
    }
    let capture: MediaStream;
    try {
      capture = await navigator.mediaDevices.getDisplayMedia({
        video: { frameRate: 30 },
        audio: false,
      });
    } catch {
      return; // user cancelled the picker
    }

    displayStream = capture;
    const videoTrack = capture.getVideoTracks()[0];

    // Add track to WebRTC mesh — peers get full-quality video P2P.
    rtcMesh?.addTrack(videoTrack);

    // Board tile placeholder for positioning + late-join snapshot.
    const id = crypto.randomUUID();
    const [x, y] = nextBoardPos();
    const placeholder: BoardItem = {
      id,
      kind: "stream",
      x,
      y,
      w: 480,
      h: 270,
      dataUrl: "",
    };

    // Low-fps WS frame fallback for board tile preview.
    const controller = await startScreenShare(
      (bytes) => {
        setStreamFrame(id, bytes);
        srocket?.send({ streamFrame: [id, bytes] });
      },
      () => stopStream(),
    );
    if (!controller) {
      capture.getTracks().forEach((t) => t.stop());
      return;
    }

    videoTrack.addEventListener("ended", () => stopStream());

    stream = controller;
    myStreamId = id;
    streamActive = true;
    upsertBoardItem(placeholder);
    srocket?.send({ boardPut: placeholder });
  }

  function stopStream() {
    stream?.stop();
    stream = null;
    streamActive = false;
    if (displayStream) {
      for (const track of displayStream.getTracks()) {
        rtcMesh?.removeTrack(track);
        track.stop();
      }
      displayStream = null;
    }
    if (myStreamId) {
      srocket?.send({ boardDelete: myStreamId });
      removeBoardItem(myStreamId);
      myStreamId = null;
    }
  }

  // Camera toggle: getUserMedia video → WebRTC mesh (P2P video tiles).
  async function handleCamera() {
    if (cameraActive && cameraStream) {
      for (const track of cameraStream.getTracks()) {
        rtcMesh?.removeTrack(track);
        track.stop();
      }
      cameraStream = null;
      cameraActive = false;
      return;
    }
    try {
      cameraStream = await navigator.mediaDevices.getUserMedia({
        video: { width: 320, height: 240, frameRate: 15 },
        audio: false,
      });
      for (const track of cameraStream.getVideoTracks()) {
        rtcMesh?.addTrack(track);
      }
      cameraActive = true;
      // Auto-enable mic on a video call so peers can talk right away.
      if (!micRecording) {
        await handleMicDown();
      }
    } catch {
      makeToast({ kind: "error", message: "Camera blocked." });
    }
  }

  // Wait for the DOM to settle before measuring terminal geometry. xterm
  // recalculates char metrics / rendered size asynchronously relative to Svelte
  // event handling and browser layout — so a font-size change followed by an
  // immediate Tile/Fit can read the PREVIOUS offsetWidth/offsetHeight for one
  // frame and compute a too-tight grid. tick() flushes pending Svelte updates;
  // two rAFs let the browser finish layout + paint so measurements are current.
  async function settleLayout() {
    await tick();
    await new Promise<void>((r) => requestAnimationFrame(() => r()));
    await new Promise<void>((r) => requestAnimationFrame(() => r()));
  }

  // The visible viewport as a world rect — the "screen" that Rectangle-style
  // snap targets map onto. Inverts normalizePosition (world = center + page/zoom
  // - offset): the top-left visible world point is (center - offset) and the
  // visible span is (innerW, innerH) / zoom.
  function visibleWorldRect(): ViewRect {
    const [ox, oy] = getConstantOffset();
    return {
      x: center[0] - ox,
      y: center[1] - oy,
      w: window.innerWidth / zoom,
      h: window.innerHeight / zoom,
    };
  }

  function terminalFootprint(id: number, ws: WsWinsize): ViewRect {
    const el = termWrappers[id];
    return {
      x: ws.x,
      y: ws.y,
      w: el && el.offsetWidth > 0 ? el.offsetWidth : ws.cols * 9.6 + 36,
      h: el && el.offsetHeight > 0 ? el.offsetHeight : ws.rows * 19 + 60,
    };
  }

  function rectsClose(a: ViewRect, b: ViewRect, tolerance = 36) {
    return (
      Math.abs(a.x - b.x) <= tolerance &&
      Math.abs(a.y - b.y) <= tolerance &&
      Math.abs(a.w - b.w) <= tolerance &&
      Math.abs(a.h - b.h) <= tolerance
    );
  }

  function activeTerminalId() {
    const live = focused[0];
    if (shells.some(([sid]) => sid === live)) return live;
    if (shells.some(([sid]) => sid === lastFocused)) return lastFocused;
    return shells[0]?.[0] ?? null;
  }

  function shortcutTargetId() {
    const live = focused[0];
    if (shells.some(([sid]) => sid === live)) return live;
    if (shells.some(([sid]) => sid === lastFocused)) return lastFocused;
    return null;
  }

  function handleShortcutSnap(id: number, action: SnapShortcutAction) {
    if (action === "restore") restoreSnap(id);
    else void applySnap(id, action);
  }

  function cycleSnapAction(id: number, requested: SnapAction, current: ViewRect) {
    const history = snapHistory[id];
    if (!history || !rectsClose(history.rect, current)) return requested;

    const advance = (cycle: SnapAction[]) => {
      const idx = cycle.indexOf(history.action);
      return idx === -1 ? requested : cycle[(idx + 1) % cycle.length];
    };

    if (requested === "firstThird") {
      return advance(["firstThird", "centerThird", "lastThird"]);
    }
    if (requested === "lastThird") {
      return advance(["lastThird", "centerThird", "firstThird"]);
    }
    if (requested === "firstTwoThirds") {
      return advance(["firstTwoThirds", "lastTwoThirds"]);
    }
    if (requested === "lastTwoThirds") {
      return advance(["lastTwoThirds", "firstTwoThirds"]);
    }
    return requested;
  }

  function recordSnap(
    id: number,
    ws: WsWinsize,
    action: SnapAction,
    previousRect: ViewRect,
    targetRect: ViewRect,
  ) {
    const history = snapHistory[id];
    if (!history || !rectsClose(history.rect, previousRect)) {
      snapRestore = { ...snapRestore, [id]: { ...ws } };
    }
    snapHistory = { ...snapHistory, [id]: { action, rect: targetRect } };
  }

  function clearLayoutMode() {
    layoutModeId = null;
    if (layoutModeTimer) {
      clearTimeout(layoutModeTimer);
      layoutModeTimer = null;
    }
  }

  function refreshLayoutModeTimeout() {
    if (layoutModeTimer) clearTimeout(layoutModeTimer);
    layoutModeTimer = setTimeout(() => clearLayoutMode(), 15000);
  }

  function enterLayoutMode(id = activeTerminalId()) {
    if (!canEdit) {
      makeToast({ kind: "info", message: "Read-only mode." });
      return;
    }
    if (id === null) {
      makeToast({ kind: "info", message: "Tap a terminal first." });
      return;
    }
    layoutModeId = id;
    refreshLayoutModeTimeout();
    makeToast({
      kind: "info",
      message: "Layout mode: arrows/U/I/J/K/F/C/1/2/3, Esc exits.",
    });
  }

  function layoutKeyAction(event: KeyboardEvent): SnapAction | "restore" | null {
    if (event.altKey || event.ctrlKey || event.metaKey) return null;
    switch (event.key) {
      case "ArrowLeft":
        return "leftHalf";
      case "ArrowRight":
        return "rightHalf";
      case "ArrowUp":
        return "topHalf";
      case "ArrowDown":
        return "bottomHalf";
      case "u":
      case "U":
        return "topLeft";
      case "i":
      case "I":
        return "topRight";
      case "j":
      case "J":
        return "bottomLeft";
      case "k":
      case "K":
        return "bottomRight";
      case "f":
      case "F":
        return "maximize";
      case "c":
      case "C":
        return "center";
      case "1":
        return "firstThird";
      case "2":
        return "centerThird";
      case "3":
        return "lastThird";
      case "r":
      case "R":
        return "restore";
      default:
        return null;
    }
  }

  // Snap one terminal into a region of the visible viewport (Rectangle-style),
  // resizing its rows/cols to fill the target. Only the final shared `move` is
  // sent; gated by canEdit like every other layout mutation.
  async function applySnap(
    id: number,
    action: SnapAction,
    settle = true,
    options: { cycle?: boolean } = {},
  ) {
    if (!canEdit) return;
    if (settle) await settleLayout(); // don't measure stale geometry after a font-size change
    if (!canEdit) return;
    const ws = shells.find(([sid]) => sid === id)?.[1];
    if (!ws) return;
    const view = visibleWorldRect();
    // Current footprint — real when laid out, cols×cell estimate as fallback.
    const currentRect = terminalFootprint(id, ws);
    const resolvedAction =
      options.cycle === false ? action : cycleSnapAction(id, action, currentRect);
    const rawTarget = computeSnapTarget(resolvedAction, view, currentRect);
    const gap =
      resolvedAction === "center" ||
      resolvedAction === "almostMaximize" ||
      resolvedAction === "maximizeHeight"
        ? 0
        : $settings.snapGap;
    const target = applySnapGap(
      rawTarget,
      gap,
      snapSharedEdges(resolvedAction, view),
    );
    // Measure this terminal's rendered cell size (post-zoom screen px -> world
    // px), the same basis tileWindows()/fitToContent() use.
    let cellW = 9.6;
    let cellH = 19;
    const screenEl = termElements[id]?.querySelector(
      ".xterm-screen",
    ) as HTMLElement | null;
    if (screenEl && zoom && ws.cols && ws.rows) {
      const r = screenEl.getBoundingClientRect();
      if (r.width && r.height) {
        const measuredW = r.width / ws.cols / zoom;
        const measuredH = r.height / ws.rows / zoom;
        if (
          Number.isFinite(measuredW) &&
          Number.isFinite(measuredH) &&
          measuredW > 2 &&
          measuredH > 6
        ) {
          cellW = measuredW;
          cellH = measuredH;
        }
      }
    }
    const CHROME_W = 36;
    const CHROME_H = 60;
    const cols = clampInt(
      (target.w - CHROME_W) / cellW,
      TERM_MIN_COLS,
      TERM_MAX_COLS,
    );
    const rows = clampInt(
      (target.h - CHROME_H) / cellH,
      TERM_MIN_ROWS,
      TERM_MAX_ROWS,
    );
    srocket?.send({
      move: [
        id,
        { ...ws, x: Math.round(target.x), y: Math.round(target.y), cols, rows },
      ],
    });
    recordSnap(id, ws, resolvedAction, currentRect, target);
  }

  function restoreSnap(id: number) {
    if (!canEdit) return;
    const previous = snapRestore[id];
    if (!previous) {
      makeToast({ kind: "info", message: "No saved layout to restore." });
      return;
    }
    srocket?.send({ move: [id, previous] });
    const { [id]: _restore, ...restRestore } = snapRestore;
    const { [id]: _history, ...restHistory } = snapHistory;
    snapRestore = restRestore;
    snapHistory = restHistory;
  }

  function handleSnapButton(id: number, action: string) {
    if (action === "restore") {
      restoreSnap(id);
    } else if (action === "layoutMode") {
      enterLayoutMode(id);
    } else if (isSnapAction(action)) {
      void applySnap(id, action);
    }
  }

  // Tile all open terminals into a uniform layout and recenter on it.
  // mode: "grid" (auto), "2col", "3col", "rows" (stacked), "cols" (side-by-side),
  // or a number = explicit column count.
  async function tileWindows(mode: string | number = "grid") {
    if (!canEdit) return;
    if (shells.length === 0) return;
    // Settle layout first so we never measure stale geometry (e.g. right after a
    // font-size change). Cheap two-frame delay; keeps the real-footprint basis.
    await settleLayout();
    // Re-read shells AFTER the await — settleLayout yields, so a terminal may have
    // opened/closed during those two frames. Deriving n (and every shells access
    // below) from the post-settle snapshot keeps the grid math in bounds: a stale
    // larger n would overrun rowH/colW → NaN coords; an emptied list would throw.
    const n = shells.length;
    if (n === 0) return;

    let nCols: number;
    if (typeof mode === "number") nCols = Math.max(1, Math.min(mode, n));
    else if (mode === "rows") nCols = 1;
    else if (mode === "cols") nCols = n;
    else if (mode === "2col") nCols = 2;
    else if (mode === "3col") nCols = 3;
    else nCols = n <= 4 ? 2 : 3; // grid (auto)
    const nRows = Math.ceil(n / nCols);

    // Measure the ACTUAL rendered cell size from a live terminal so windows
    // never overlap (the previous fixed estimate was too small for tall ones).
    // getBoundingClientRect is post-zoom screen px → divide by zoom for world px.
    let cellW = 9.6;
    let cellH = 19;
    const firstId = shells[0][0];
    const screenEl = termElements[firstId]?.querySelector(
      ".xterm-screen",
    ) as HTMLElement | null;
    if (screenEl && zoom) {
      const ws0 = shells[0][1];
      const r = screenEl.getBoundingClientRect();
      if (r.width && r.height && ws0.cols && ws0.rows) {
        cellW = r.width / ws0.cols / zoom;
        cellH = r.height / ws0.rows / zoom;
      }
    }
    const CHROME_W = 36; // borders + horizontal padding (world px)
    const CHROME_H = 60; // title bar + vertical padding (world px)
    const GAP = 48;

    // Keep each terminal's OWN size — if the user resized a window, honour it
    // instead of snapping back to the default ROWS×COLS. We only reposition.
    // Box = the terminal's REAL rendered footprint (offsetWidth/Height, world px,
    // zoom-independent) — the same basis the upstream arrange.ts uses, so the
    // grid never overlaps and is identical on every device. The cols×cell
    // estimate is only a fallback for a window that hasn't laid out yet.
    const boxes = shells.map(([id, ws]) => {
      const el = termWrappers[id];
      return {
        id,
        ws,
        w: el && el.offsetWidth > 0 ? el.offsetWidth : ws.cols * cellW + CHROME_W,
        h: el && el.offsetHeight > 0 ? el.offsetHeight : ws.rows * cellH + CHROME_H,
      };
    });

    // Per-column width and per-row height = the largest terminal in that
    // column/row, so variable-size windows line up cleanly and never overlap.
    const colW = new Array(nCols).fill(0);
    const rowH = new Array(nRows).fill(0);
    boxes.forEach((b, i) => {
      const col = i % nCols;
      const row = Math.floor(i / nCols);
      colW[col] = Math.max(colW[col], b.w);
      rowH[row] = Math.max(rowH[row], b.h);
    });

    // Cumulative offsets (each track + a gap after it).
    const colX = new Array(nCols);
    const rowY = new Array(nRows);
    let accX = 0;
    for (let c = 0; c < nCols; c++) {
      colX[c] = accX;
      accX += colW[c] + GAP;
    }
    let accY = 0;
    for (let r = 0; r < nRows; r++) {
      rowY[r] = accY;
      accY += rowH[r] + GAP;
    }
    const gridW = Math.max(0, accX - GAP);
    const gridH = Math.max(0, accY - GAP);

    boxes.forEach((b, i) => {
      const col = i % nCols;
      const row = Math.floor(i / nCols);
      const x = Math.round(-gridW / 2 + colX[col]);
      const y = Math.round(-gridH / 2 + rowY[row]);
      // Preserve rows/cols — only the position changes.
      srocket?.send({ move: [b.id, { ...b.ws, x, y }] });
    });
    // Recenter + zoom so the whole grid (centered on origin) fits. touchZoom
    // owns center/zoom, so assigning them directly would get overwritten.
    const fit = Math.min(
      window.innerWidth / (gridW + 160),
      window.innerHeight / (gridH + 160),
    );
    touchZoom.moveTo([0, 0], Math.max(0.25, Math.min(1, fit)));
  }

  // Fit every window/item into THIS viewport, centered — a local view change
  // (zoom + pan only), so it never moves the shared world positions and never
  // disturbs other devices. This is the responsive "see everything, centered"
  // that makes one shared layout look right on any screen size. (Bo 2026-06-13)
  async function fitToContent() {
    // Settle layout first (see settleLayout) so the bbox is measured from current
    // terminal footprints, not stale ones after a font-size change.
    await settleLayout();
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    const add = (x: number, y: number, w: number, h: number) => {
      minX = Math.min(minX, x);
      minY = Math.min(minY, y);
      maxX = Math.max(maxX, x + w);
      maxY = Math.max(maxY, y + h);
    };
    // Derive each terminal's world box from its rows/cols + cell size (measured
    // from a live terminal, with a fallback estimate) instead of reading
    // offsetWidth. Shell data is available immediately, so the fit no longer
    // depends on DOM render timing — that timing was the bug that left the view
    // stuck at the origin when a terminal hadn't laid out yet.
    let cellW = 9.6;
    let cellH = 19;
    if (shells.length) {
      const ws0 = shells[0][1];
      const screenEl = termElements[shells[0][0]]?.querySelector(
        ".xterm-screen",
      ) as HTMLElement | null;
      if (screenEl && zoom && ws0.cols && ws0.rows) {
        const r = screenEl.getBoundingClientRect();
        if (r.width && r.height) {
          cellW = r.width / ws0.cols / zoom;
          cellH = r.height / ws0.rows / zoom;
        }
      }
    }
    const CHROME_W = 36;
    const CHROME_H = 60;
    for (const [id, ws] of shells) {
      // Real rendered footprint when the window has laid out (exact, matches the
      // grid + arrange.ts); cols×cell estimate only as an early-load fallback.
      const el = termWrappers[id];
      const w = el && el.offsetWidth > 0 ? el.offsetWidth : ws.cols * cellW + CHROME_W;
      const h = el && el.offsetHeight > 0 ? el.offsetHeight : ws.rows * cellH + CHROME_H;
      add(ws.x, ws.y, w, h);
    }
    for (const it of boardItems) {
      if (it.kind === "doc" || it.kind === "lock" || it.kind === "label") continue;
      if (!it.w || !it.h) continue;
      add(it.x, it.y, it.w, it.h);
    }
    if (!isFinite(minX)) {
      touchZoom.moveTo([0, 0], INITIAL_ZOOM); // empty board
      return;
    }
    const PAD = 80; // breathing room around the content (screen px)
    const w = maxX - minX;
    const h = maxY - minY;
    const fit = Math.min(
      window.innerWidth / (w + PAD * 2),
      window.innerHeight / (h + PAD * 2),
    );
    const z = Math.max(0.2, Math.min(1, fit));
    // Solve for the touchZoom center that lands the bbox centre at the viewport
    // centre. World→screen is screen = (world - center + constantOffset)*zoom
    // (see normalizePosition); the canvas origin is offset by CONSTANT_OFFSET
    // (the terminal spawn point), NOT the viewport centre — so passing the bbox
    // centre directly left the content stuck up-left. Invert for `center`.
    const off = getConstantOffset();
    const bcx = (minX + maxX) / 2;
    const bcy = (minY + maxY) / 2;
    const cx = bcx + off[0] - window.innerWidth / (2 * z);
    const cy = bcy + off[1] - window.innerHeight / (2 * z);
    touchZoom.moveTo([cx, cy], z);
  }

  // Center button → fit everything centered (was: reset to origin).
  function handleCenter() {
    fitToContent();
  }

  // Clear the board — remove notes/images/videos/streams (keeps terminals and
  // the shared document).
  function handleClear() {
    if (!canEdit) return;
    for (const item of boardItems) {
      // Keep singletons that aren't board "content": doc, lock state, labels.
      if (item.kind === "doc" || item.kind === "lock" || item.kind === "label")
        continue;
      srocket?.send({ boardDelete: item.id });
      removeBoardItem(item.id);
    }
  }

  function handleBoardMove(id: string, x: number, y: number) {
    if (!canEdit) return;
    const item = boardItems.find((it) => it.id === id);
    if (item) upsertBoardItem({ ...item, x, y });
    srocket?.send({ boardMove: [id, x, y] });
  }

  function handleBoardResize(id: string, w: number, h: number) {
    if (!canEdit) return;
    const item = boardItems.find((it) => it.id === id);
    if (!item) return;
    const resized = { ...item, w, h };
    upsertBoardItem(resized);
    srocket?.send({ boardPut: resized }); // no boardResize msg — sync full item
  }

  function handleBoardDelete(id: string) {
    if (!canEdit) return;
    removeBoardItem(id);
    srocket?.send({ boardDelete: id });
  }

  onDestroy(() => {
    stream?.stop();
    rtcMesh?.dispose();
    touchZoom?.destroy();
    micStream?.getTracks().forEach((t) => t.stop());
    cameraStream?.getTracks().forEach((t) => t.stop());
    for (const audio of Object.values(remoteAudios)) audio.pause();
    for (const url of Object.values(streamSrcs)) URL.revokeObjectURL(url);
    // Drop the board-color we painted on the document so SPA nav back to the
    // landing page doesn't inherit it (falls back to the stylesheet).
    if (typeof document !== "undefined") {
      document.documentElement.style.backgroundColor = "";
      document.body.style.backgroundColor = "";
    }
  });
</script>

<!-- Wheel handler stops native macOS Chrome zooming on pinch. -->
<main
  class="p-2 sm:p-8"
  class:cursor-nwse-resize={resizing !== -1}
  style:background-color={$settings.background}
  on:wheel={(event) => event.preventDefault()}
>
  <div
    class="absolute top-8 inset-x-0 flex justify-center pointer-events-none z-10 transition-all duration-300 ease-out"
    class:opacity-0={!toolbarVisible}
    class:-translate-y-[150%]={!toolbarVisible}
    on:pointerenter={showToolbar}
  >
    <Toolbar
      {connected}
      {newMessages}
      {hasWriteAccess}
      {micRecording}
      {cameraActive}
      {boardLocked}
      {lockedForMe}
      {broadcastMode}
      numpadOpen={showNumpad}
      on:create={handleCreate}
      on:lock={toggleLock}
      on:broadcast={() => (broadcastMode = !broadcastMode)}
      on:snippets={() => (showSnippets = !showSnippets)}
      on:tile={({ detail }) => tileWindows(detail)}
      on:center={handleCenter}
      on:clear={handleClear}
      on:note={addNote}
      on:video={handleVideoPick}
      on:files={() => (showExplorer = !showExplorer)}
      on:numpad={() => (showNumpad = !showNumpad)}
      on:doc={() => (showDoc = !showDoc)}
      on:chat={() => {
        showChat = !showChat;
        newMessages = false;
      }}
      on:settings={() => {
        settingsOpen = true;
      }}
      on:networkInfo={() => {
        showNetworkInfo = !showNetworkInfo;
      }}
      on:youtube={() => (showYouTube = !showYouTube)}
      on:micDown={handleMicDown}
      on:image={handleImage}
      on:stream={handleStream}
      on:camera={handleCamera}
    />

    {#if showNetworkInfo}
      <div class="absolute top-20 left-2 right-2 sm:left-auto sm:right-auto translate-x-0 sm:translate-x-[116.5px] max-w-[calc(100vw-1rem)] sm:max-w-none">
        <NetworkInfo
          status={connected
            ? "connected"
            : exitReason
            ? "no-shell"
            : "no-server"}
          serverLatency={integerMedian(serverLatencies)}
          shellLatency={integerMedian(shellLatencies)}
        />
      </div>
    {/if}
  </div>

  <!--
    Peek-handle: when the auto-hiding toolbar is tucked away, a clearly-visible
    pill stays pinned to the top edge so you always know where it lives — tap or
    hover it to bring the toolbar back. Fades out while the toolbar is showing.
    (Bo 2026-06-13: made bold + a chevron — the old faint 25% line was invisible.)
  -->
  <button
    class="absolute top-1.5 left-1/2 -translate-x-1/2 z-20 px-3 py-1 flex items-center gap-1.5 rounded-full bg-zinc-900/80 ring-1 ring-white/25 shadow-lg backdrop-blur-sm cursor-pointer group transition-all duration-300 ease-out hover:bg-zinc-800/90 hover:ring-white/50"
    class:opacity-0={toolbarVisible}
    class:pointer-events-none={toolbarVisible}
    on:pointerenter={showToolbar}
    on:click={showToolbar}
    aria-label="Show toolbar"
    title="Show toolbar"
  >
    <span
      class="w-7 h-1 rounded-full bg-white/80 group-hover:w-9 group-hover:bg-white transition-all duration-200"
    />
    <span class="text-white/80 text-[11px] leading-none group-hover:text-white">⌄</span>
  </button>

  <!-- Lock banner: tells locked-out viewers why the board is read-only. -->
  {#if lockedForMe}
    <div
      class="absolute bottom-4 left-1/2 -translate-x-1/2 z-20 flex items-center gap-2 px-4 py-2 rounded-full bg-amber-500/95 text-zinc-900 text-sm font-semibold shadow-lg pointer-events-none"
    >
      🔒 Board locked by {boardLock?.ownerName ?? "someone"} — view only
    </div>
  {/if}

  <!-- Broadcast banner: loud warning that keystrokes hit every terminal. -->
  {#if broadcastMode}
    <div
      class="absolute top-14 left-1/2 -translate-x-1/2 z-20 flex items-center gap-2 px-4 py-2 rounded-full bg-red-600/95 text-white text-sm font-semibold shadow-lg pointer-events-none animate-pulse"
    >
      📡 Broadcast ON — typing goes to ALL terminals
    </div>
  {/if}

  {#if layoutModeId !== null}
    <div
      class="absolute top-14 left-1/2 -translate-x-1/2 z-30 flex items-center gap-2 px-4 py-2 rounded-full bg-indigo-600/95 text-white text-sm font-semibold shadow-lg pointer-events-none"
    >
      ⌨ Layout terminal {labels[layoutModeId] ?? layoutModeId} · Esc exits
    </div>
  {/if}

  {#if showChat}
    <div
      class="absolute flex flex-col justify-end inset-y-4 left-2 right-2 sm:left-auto sm:right-4 w-auto sm:w-80 pointer-events-none z-10"
    >
      <Chat
        {userId}
        messages={chatMessages}
        on:chat={(event) => srocket?.send({ chat: event.detail })}
        on:close={() => (showChat = false)}
      />
    </div>
  {/if}

  <Settings open={settingsOpen} on:close={() => (settingsOpen = false)} />

  <YouTubePopup open={showYouTube} on:close={() => (showYouTube = false)} />

  {#if showExplorer}
    <FileExplorer
      on:close={() => (showExplorer = false)}
      on:open={({ detail }) => openFile(detail)}
    />
  {/if}

  <SnippetBar
    open={showSnippets}
    on:paste={({ detail }) => pasteSnippet(detail)}
    on:close={() => (showSnippets = false)}
  />

  {#if showNumpad}
    <Numpad
      on:press={({ detail }) => handleNumpadPress(detail)}
      on:snap={({ detail }) => handleNumpadSnap(detail)}
      on:close={() => (showNumpad = false)}
    />
  {/if}

  {#if showDoc}
    <MarkdownDoc
      text={docText}
      readonly={!canEdit}
      on:edit={({ detail }) => handleDocEdit(detail)}
      on:close={() => (showDoc = false)}
    />
  {/if}

  {#if showFileView}
    <MarkdownDoc
      text={fileViewText}
      readonly={true}
      on:close={() => (showFileView = false)}
    />
  {/if}

  {#if cameraActive && cameraStream}
    <CameraPreview stream={cameraStream} on:close={handleCamera} />
  {/if}

  {#each Object.entries(remoteVideos) as [uid, stream], i (uid)}
    <CameraPreview
      {stream}
      label={users.find(([u]) => String(u) === uid)?.[1]?.name ?? "Peer"}
      mirror={false}
      closable={false}
      index={i + (cameraActive ? 1 : 0)}
    />
  {/each}

  <ChooseName />

  <!--
    Dotted circle background appears underneath the rest of the elements, but
    moves and zooms with the fabric of the canvas.
  -->
  <div
    class="absolute inset-0 -z-10"
    style:background-image="radial-gradient(#282828 {zoom * 0.8}px, transparent 0)"
    style:background-size="{24 * zoom}px {24 * zoom}px"
    style:background-position="{-zoom * center[0]}px {-zoom * center[1]}px"
  />

  <div class="flex items-center gap-2 py-2 flex-wrap">
    {#if exitReason !== null}
      <div
        class="px-2.5 py-1 rounded-full text-xs font-medium bg-red-900/60 text-red-300 border border-red-800/40"
      >
        {exitReason}
      </div>
    {:else if connected}
      <div
        class="px-2.5 py-1 rounded-full text-xs font-medium bg-emerald-900/50 text-emerald-300 border border-emerald-800/40 flex items-center gap-1.5"
      >
        <div class="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse" />
        Connected
      </div>
      {#if userId && hasWriteAccess === false}
        <div
          class="px-2.5 py-1 rounded-full text-xs font-medium bg-yellow-900/50 text-yellow-300 border border-yellow-800/40 flex items-center gap-1"
        >
          <EyeIcon size="12" />
          Read-only
        </div>
      {/if}
    {:else}
      <div
        class="px-2.5 py-1 rounded-full text-xs font-medium bg-yellow-900/50 text-yellow-300 border border-yellow-800/40"
      >
        Connecting…
      </div>
    {/if}
  </div>

  <!-- Online users — vertical column down the left edge. -->
  {#if users.length > 0}
    <div class="fixed left-3 top-24 z-30 max-h-[70vh] overflow-y-auto pointer-events-auto">
      <NameList {users} vertical />
    </div>
  {/if}

  <div class="absolute inset-0 overflow-hidden touch-none" bind:this={fabricEl}>
    <Board
      items={boardItems}
      {streamSrcs}
      {center}
      {zoom}
      hasWriteAccess={canEdit}
      offsetLeftCss={OFFSET_LEFT_CSS}
      offsetTopCss={OFFSET_TOP_CSS}
      offsetTransformOriginCss={OFFSET_TRANSFORM_ORIGIN_CSS}
      {normalizePosition}
      {snapTargets}
      extraGuidesV={termGuidesV}
      extraGuidesH={termGuidesH}
      on:move={({ detail }) =>
        handleBoardMove(detail.id, detail.x, detail.y)}
      on:resize={({ detail }) =>
        handleBoardResize(detail.id, detail.w, detail.h)}
      on:delete={({ detail }) => handleBoardDelete(detail)}
      on:edit={({ detail }) => handleNoteEdit(detail.id, detail.text)}
    />

    {#if edgeSnapPreview}
      <div
        class="pointer-events-none absolute z-20 rounded-xl border border-indigo-300/80 bg-indigo-500/20 shadow-[0_0_0_1px_rgba(99,102,241,0.28),0_16px_48px_rgba(79,70,229,0.22)]"
        style:left={OFFSET_LEFT_CSS}
        style:top={OFFSET_TOP_CSS}
        style:width={`${Math.max(0, edgeSnapPreview.w)}px`}
        style:height={`${Math.max(0, edgeSnapPreview.h)}px`}
        style:transform-origin={OFFSET_TRANSFORM_ORIGIN_CSS}
        use:slide={{
          x: edgeSnapPreview.x,
          y: edgeSnapPreview.y,
          center,
          zoom,
          immediate: true,
        }}
      />
    {/if}

    {#each shells as [id, winsize] (id)}
      {@const ws = id === moving ? movingSize : winsize}
      <div
        class="absolute"
        style:left={OFFSET_LEFT_CSS}
        style:top={OFFSET_TOP_CSS}
        style:transform-origin={OFFSET_TRANSFORM_ORIGIN_CSS}
        transition:fade|local
        use:slide={{ x: ws.x, y: ws.y, center, zoom, immediate: id === moving }}
        bind:this={termWrappers[id]}
      >
        <XTerm
          rows={ws.rows}
          cols={ws.cols}
          bind:write={writers[id]}
          bind:termEl={termElements[id]}
          label={labels[id] ?? ""}
          canRename={canEdit}
          snapPadOpen={canEdit && snapPadFor === id}
          on:rename={({ detail }) => handleRename(id, detail)}
          on:data={({ detail: data }) => routeInput(id, data)}
          on:close={() => canEdit && srocket?.send({ close: id })}
          on:shrink={() => {
            if (!canEdit) return;
            const rows = Math.max(ws.rows - 4, TERM_MIN_ROWS);
            const cols = Math.max(ws.cols - 10, TERM_MIN_COLS);
            if (rows !== ws.rows || cols !== ws.cols) {
              srocket?.send({ move: [id, { ...ws, rows, cols }] });
            }
          }}
          on:expand={() => {
            if (!canEdit) return;
            const rows = ws.rows + 4;
            const cols = ws.cols + 10;
            srocket?.send({ move: [id, { ...ws, rows, cols }] });
          }}
          on:preset={({ detail: { cols, rows } }) => {
            if (!canEdit) return;
            const r = Math.max(rows, TERM_MIN_ROWS);
            const c = Math.max(cols, TERM_MIN_COLS);
            srocket?.send({ move: [id, { ...ws, rows: r, cols: c }] });
          }}
          on:snap={({ detail }) => handleSnapButton(id, detail)}
          on:showSnapPad={() => {
            if (canEdit) snapPadFor = id;
          }}
          on:hideSnapPad={() => {
            if (snapPadFor === id) snapPadFor = null;
          }}
          on:bringToFront={() => {
            if (!canEdit) return;
            showNetworkInfo = false;
            srocket?.send({ move: [id, null] });
          }}
          on:startMove={({ detail: event }) => handleStartMove(id, ws, event)}
          on:focus={() => {
            if (hasWriteAccess === false) return;
            focused = [...focused, id];
            lastFocused = id; // remember for snippet paste after panel clicks
            if (canEdit) snapPadFor = id;
          }}
          on:blur={() => {
            focused = focused.filter((i) => i !== id);
            if (snapPadFor === id) snapPadFor = null;
          }}
        />

        <!-- User avatars -->
        <div class="absolute bottom-2.5 right-2.5 pointer-events-none">
          <Avatars
            users={users.filter(
              ([uid, user]) => uid !== userId && user.focus === id,
            )}
          />
        </div>

        <!-- Interactable element for resizing (mouse + touch via pointer events) -->
        <div
          class="terminal-resize-handle absolute w-5 h-5 -bottom-1 -right-1 cursor-nwse-resize touch-none"
          on:pointerdown={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (!canEdit) return;
            const canvasEl = termElements[id].querySelector(".xterm-screen");
            if (canvasEl) {
              resizing = id;
              resizingPointerId = event.pointerId ?? null;
              const r = canvasEl.getBoundingClientRect();
              resizingOrigin = [event.pageX - r.width, event.pageY - r.height];
              resizingCell = [r.width / ws.cols, r.height / ws.rows];
              resizingSize = ws;
            }
          }}
        />
      </div>
    {/each}

    {#each users.filter(([id, user]) => id !== userId && user.cursor !== null) as [id, user] (id)}
      <div
        class="absolute"
        style:left={OFFSET_LEFT_CSS}
        style:top={OFFSET_TOP_CSS}
        style:transform-origin={OFFSET_TRANSFORM_ORIGIN_CSS}
        transition:fade|local={{ duration: 200 }}
        use:slide={{
          x: user.cursor?.[0] ?? 0,
          y: user.cursor?.[1] ?? 0,
          center,
          zoom,
        }}
      >
        <LiveCursor {user} />
      </div>
    {/each}
  </div>
</main>
