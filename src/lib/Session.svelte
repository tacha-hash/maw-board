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
    createVoiceCapture,
    playVoice,
    readImageFile,
    startScreenShare,
    type StreamController,
    type VoiceController,
  } from "./board";
  import { RtcMesh } from "./rtc";
  import { makeToast } from "./toast";
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
  let movingOrigin = [0, 0]; // Coordinates of mouse at origin when drag started.
  let movingSize: WsWinsize; // New [x, y] position of the dragged terminal.
  let movingIsDone = false; // Moving finished but hasn't been acknowledged.

  let resizing = -1; // Terminal ID that is being resized.
  let resizingOrigin = [0, 0]; // Coordinates of top-left origin when resize started.
  let resizingCell = [0, 0]; // Pixel dimensions of a single terminal cell.
  let resizingSize: WsWinsize; // Last resize message sent.

  let chatMessages: ChatMessage[] = [];
  let newMessages = false;
  let initialShellsReceived = false;

  // ── maw share workboard extensions ──
  let boardItems: BoardItem[] = [];
  // Live screen-share frames as object URLs, keyed by board item id.
  let streamSrcs: Record<string, string> = {};
  let voice: VoiceController | null = null;
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
          shells = message.shells;
          if (movingIsDone) {
            moving = -1;
          }
          // Auto-create one terminal when joining a fresh empty session.
          if (!initialShellsReceived && message.shells.length === 0 && hasWriteAccess !== false) {
            handleCreate();
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
            if (to === userId) rtcMesh?.handleSignal(from, payload);
          } else {
            const [from, payload] = sig as unknown as [number, string];
            rtcMesh?.handleSignal(from, payload);
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

  async function handleCreate() {
    if (hasWriteAccess === false) {
      makeToast({
        kind: "info",
        message: "You are in read-only mode and cannot create new terminals.",
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
    srocket?.send({ create: [x, y] });
    touchZoom.moveTo([x, y], INITIAL_ZOOM);
  }

  async function handleInput(id: number, data: Uint8Array) {
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
        const [x, y] = normalizePosition(event);
        movingSize = {
          ...movingSize,
          x: Math.round(x - movingOrigin[0]),
          y: Math.round(y - movingOrigin[1]),
        };
        sendMove({ move: [moving, movingSize] });
      }

      if (resizing !== -1) {
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
        movingIsDone = true;
        sendMove.cancel();
        srocket?.send({ move: [moving, movingSize] });
      }

      if (resizing !== -1) {
        resizing = -1;
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
    document.body.addEventListener("pointerleave", handleMouseEnd);
    return () => {
      window.removeEventListener("pointermove", handleMouse);
      window.removeEventListener("pointerup", handleMouseEnd);
      document.body.removeEventListener("pointerleave", handleMouseEnd);
    };
  });

  let focused: number[] = [];
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
  function addImage(file: File) {
    if (hasWriteAccess === false) return;
    readImageFile(file, (dataUrl, w, h) => {
      const [x, y] = nextBoardPos();
      const item: BoardItem = {
        id: crypto.randomUUID(),
        kind: "image",
        x,
        y,
        w,
        h,
        dataUrl,
      };
      upsertBoardItem(item);
      srocket?.send({ boardPut: item });
    });
  }

  // Add an editable sticky note to the board.
  function addNote() {
    if (hasWriteAccess === false) return;
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

  // Persist a note's edited text to peers.
  function handleNoteEdit(id: string, text: string) {
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

  const MAX_VIDEO_BYTES = 8 * 1024 * 1024; // 8 MB — kept small so it syncs over WS

  // Add a video clip to the board (shared via boardPut as a data URL).
  function addVideoFile(file: File) {
    if (hasWriteAccess === false) return;
    if (!file.type.startsWith("video/")) return;
    if (file.size > MAX_VIDEO_BYTES) {
      makeToast({
        kind: "error",
        message: "Video too large (max 8 MB to share on the board).",
      });
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      const [x, y] = nextBoardPos();
      const item: BoardItem = {
        id: crypto.randomUUID(),
        kind: "video",
        x,
        y,
        w: 320,
        h: 240,
        dataUrl: String(reader.result),
      };
      upsertBoardItem(item);
      srocket?.send({ boardPut: item });
    };
    reader.readAsDataURL(file);
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
    window.addEventListener("paste", onPaste);
    window.addEventListener("dragover", onDragOver);
    window.addEventListener("drop", onDrop);
    return () => {
      window.removeEventListener("paste", onPaste);
      window.removeEventListener("dragover", onDragOver);
      window.removeEventListener("drop", onDrop);
    };
  });

  // Screen share: getDisplayMedia → send video track via WebRTC to peers +
  // place a board tile (WS StreamFrame fallback for board preview at ~3fps).
  let displayStream: MediaStream | null = null;

  async function handleStream() {
    if (hasWriteAccess === false) return;
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

  // Tile all open terminals into a uniform grid and recenter the view on it.
  // Columns auto-scale: ≤2 → 2 cols (1 row), ≤4 → 2 cols (2×2), else 3 cols.
  function tileWindows() {
    if (!hasWriteAccess) return;
    const n = shells.length;
    if (n === 0) return;
    const nCols = n <= 4 ? 2 : 3;
    const nRows = Math.ceil(n / nCols);
    const COLS = 90;
    const ROWS = 40;
    // Approximate rendered window footprint in world px (incl. header + gaps).
    const TERM_W = COLS * 9 + 40;
    const TERM_H = ROWS * 17 + 80;
    const gridW = nCols * TERM_W;
    const gridH = nRows * TERM_H;
    shells.forEach(([id, ws], i) => {
      const col = i % nCols;
      const row = Math.floor(i / nCols);
      const x = Math.round(-gridW / 2 + col * TERM_W);
      const y = Math.round(-gridH / 2 + row * TERM_H);
      srocket?.send({ move: [id, { ...ws, x, y, rows: ROWS, cols: COLS }] });
    });
    // Recenter + zoom so the whole grid (centered on world origin) fits in view.
    // Go through touchZoom — it owns center/zoom, so assigning them directly
    // would get overwritten on its next frame.
    const fit = Math.min(
      window.innerWidth / (gridW + 160),
      window.innerHeight / (gridH + 160),
    );
    touchZoom.moveTo([0, 0], Math.max(0.3, Math.min(1, fit)));
  }

  function handleBoardMove(id: string, x: number, y: number) {
    const item = boardItems.find((it) => it.id === id);
    if (item) upsertBoardItem({ ...item, x, y });
    srocket?.send({ boardMove: [id, x, y] });
  }

  function handleBoardDelete(id: string) {
    removeBoardItem(id);
    srocket?.send({ boardDelete: id });
  }

  onDestroy(() => {
    stream?.stop();
    rtcMesh?.dispose();
    micStream?.getTracks().forEach((t) => t.stop());
    cameraStream?.getTracks().forEach((t) => t.stop());
    for (const audio of Object.values(remoteAudios)) audio.pause();
    for (const url of Object.values(streamSrcs)) URL.revokeObjectURL(url);
  });
</script>

<!-- Wheel handler stops native macOS Chrome zooming on pinch. -->
<main
  class="p-8"
  class:cursor-nwse-resize={resizing !== -1}
  style:background-color={$settings.background}
  on:wheel={(event) => event.preventDefault()}
>
  <div
    class="absolute top-8 inset-x-0 flex justify-center pointer-events-none z-10"
  >
    <Toolbar
      {connected}
      {newMessages}
      {hasWriteAccess}
      {micRecording}
      {cameraActive}
      on:create={handleCreate}
      on:tile={tileWindows}
      on:note={addNote}
      on:video={handleVideoPick}
      on:files={() => (showExplorer = !showExplorer)}
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
      on:micDown={handleMicDown}
      on:image={handleImage}
      on:stream={handleStream}
      on:camera={handleCamera}
    />

    {#if showNetworkInfo}
      <div class="absolute top-20 translate-x-[116.5px]">
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

  {#if showChat}
    <div
      class="absolute flex flex-col justify-end inset-y-4 right-4 w-80 pointer-events-none z-10"
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

  {#if showExplorer}
    <FileExplorer on:close={() => (showExplorer = false)} />
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

    <NameList {users} />
  </div>

  <div class="absolute inset-0 overflow-hidden touch-none" bind:this={fabricEl}>
    <Board
      items={boardItems}
      {streamSrcs}
      {center}
      {zoom}
      {hasWriteAccess}
      offsetLeftCss={OFFSET_LEFT_CSS}
      offsetTopCss={OFFSET_TOP_CSS}
      offsetTransformOriginCss={OFFSET_TRANSFORM_ORIGIN_CSS}
      {normalizePosition}
      on:move={({ detail }) =>
        handleBoardMove(detail.id, detail.x, detail.y)}
      on:delete={({ detail }) => handleBoardDelete(detail)}
      on:edit={({ detail }) => handleNoteEdit(detail.id, detail.text)}
    />

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
          on:data={({ detail: data }) =>
            hasWriteAccess && handleInput(id, data)}
          on:close={() => srocket?.send({ close: id })}
          on:shrink={() => {
            if (!hasWriteAccess) return;
            const rows = Math.max(ws.rows - 4, TERM_MIN_ROWS);
            const cols = Math.max(ws.cols - 10, TERM_MIN_COLS);
            if (rows !== ws.rows || cols !== ws.cols) {
              srocket?.send({ move: [id, { ...ws, rows, cols }] });
            }
          }}
          on:expand={() => {
            if (!hasWriteAccess) return;
            const rows = ws.rows + 4;
            const cols = ws.cols + 10;
            srocket?.send({ move: [id, { ...ws, rows, cols }] });
          }}
          on:preset={({ detail: { cols, rows } }) => {
            if (!hasWriteAccess) return;
            const r = Math.max(rows, TERM_MIN_ROWS);
            const c = Math.max(cols, TERM_MIN_COLS);
            srocket?.send({ move: [id, { ...ws, rows: r, cols: c }] });
          }}
          on:bringToFront={() => {
            if (!hasWriteAccess) return;
            showNetworkInfo = false;
            srocket?.send({ move: [id, null] });
          }}
          on:startMove={({ detail: event }) => {
            if (!hasWriteAccess) return;
            const [x, y] = normalizePosition(event);
            moving = id;
            movingOrigin = [x - ws.x, y - ws.y];
            movingSize = ws;
            movingIsDone = false;
          }}
          on:focus={() => {
            if (!hasWriteAccess) return;
            focused = [...focused, id];
          }}
          on:blur={() => {
            focused = focused.filter((i) => i !== id);
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
          class="absolute w-5 h-5 -bottom-1 -right-1 cursor-nwse-resize touch-none"
          on:pointerdown={(event) => {
            event.stopPropagation();
            const canvasEl = termElements[id].querySelector(".xterm-screen");
            if (canvasEl) {
              resizing = id;
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
