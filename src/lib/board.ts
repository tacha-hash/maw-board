/**
 * @file Media-capture helpers for the maw share workboard extensions.
 *
 * Framework-agnostic logic ported from `~/.maw/plugins/ssh-share/viewer.js`:
 * push-to-talk voice capture, screen-share frame loop, and image downscaling.
 * These return raw bytes / data URLs through callbacks; Session.svelte owns the
 * srocket plumbing per the contract v2 seam.
 */

/** A push-to-talk microphone controller. */
export type VoiceController = {
  /** Begin recording a clip (no-op if already recording). */
  start(): Promise<void>;
  /** Stop recording; the finished clip is delivered via the `onClip` callback. */
  stop(): void;
  /** Whether a recording is currently in progress. */
  readonly recording: boolean;
};

/**
 * Create a push-to-talk voice capturer. Each press/release produces one
 * webm/opus clip, delivered to `onClip` as raw bytes for `{ voice: bytes }`.
 */
export function createVoiceCapture(
  onClip: (bytes: Uint8Array) => void,
): VoiceController {
  let micStream: MediaStream | null = null;
  let recorder: MediaRecorder | null = null;
  let recording = false;

  return {
    get recording() {
      return recording;
    },
    async start() {
      if (recording) return;
      if (!micStream) {
        micStream = await navigator.mediaDevices.getUserMedia({ audio: true });
      }
      const mime = MediaRecorder.isTypeSupported("audio/webm;codecs=opus")
        ? "audio/webm;codecs=opus"
        : "audio/webm";
      recorder = new MediaRecorder(micStream, { mimeType: mime });
      const chunks: Blob[] = [];
      recorder.ondataavailable = (e) => {
        if (e.data?.size) chunks.push(e.data);
      };
      recorder.onstop = async () => {
        const blob = new Blob(chunks, { type: "audio/webm" });
        if (blob.size) onClip(new Uint8Array(await blob.arrayBuffer()));
      };
      recorder.start();
      recording = true;
    },
    stop() {
      if (!recording) return;
      recording = false;
      try {
        if (recorder && recorder.state !== "inactive") recorder.stop();
      } catch {
        // already stopped
      }
    },
  };
}

/** Play back a received voice clip (webm/opus bytes from `voiceData`). */
export function playVoice(bytes: Uint8Array) {
  const url = URL.createObjectURL(new Blob([bytes], { type: "audio/webm" }));
  const audio = new Audio(url);
  audio.onended = () => URL.revokeObjectURL(url);
  audio.play().catch(() => URL.revokeObjectURL(url));
}

/** A running screen-share session. */
export type StreamController = {
  /** Stop sharing and release the display track. */
  stop(): void;
};

/**
 * Begin sharing the screen. Frames are encoded to JPEG at ~3 fps and delivered
 * to `onFrame` as raw bytes for `{ streamFrame: [id, bytes] }`. Resolves to
 * `null` if the user cancels the picker. `onEnded` fires if the browser's own
 * "Stop sharing" control ends the track.
 */
export async function startScreenShare(
  onFrame: (bytes: Uint8Array) => void,
  onEnded: () => void,
): Promise<StreamController | null> {
  let stream: MediaStream;
  try {
    stream = await navigator.mediaDevices.getDisplayMedia({
      video: { frameRate: 4 },
      audio: false,
    });
  } catch {
    return null; // user cancelled the share picker
  }

  const video = document.createElement("video");
  video.srcObject = stream;
  video.muted = true;
  await video.play();

  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d");

  const timer = window.setInterval(() => {
    const vw = video.videoWidth;
    const vh = video.videoHeight;
    if (!vw || !ctx) return;
    const scale = Math.min(1, 640 / vw);
    canvas.width = Math.round(vw * scale);
    canvas.height = Math.round(vh * scale);
    ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
    canvas.toBlob(
      async (blob) => {
        if (blob) onFrame(new Uint8Array(await blob.arrayBuffer()));
      },
      "image/jpeg",
      0.5,
    );
  }, 300);

  let stopped = false;
  function stop() {
    if (stopped) return;
    stopped = true;
    window.clearInterval(timer);
    stream.getTracks().forEach((t) => t.stop());
  }

  stream.getVideoTracks()[0].addEventListener("ended", () => {
    stop();
    onEnded();
  });

  return { stop };
}

/** Max width encoded for peers over WS (was 720 — too blurry on phone). */
export const IMAGE_SHARE_MAX_WIDTH = 1920;
/** Default tile width on the board canvas. */
export const IMAGE_TILE_MAX_WIDTH = 960;
export const IMAGE_JPEG_QUALITY = 0.92;
/** Skip broadcasting huge data URLs (mirrors video share cap pattern). */
export const IMAGE_SHARE_CAP_BYTES = 3 * 1024 * 1024;

export type ImagePayload = {
  /** Encoded data URL for peers (JPEG/PNG). */
  dataUrl: string;
  /** Natural encoded dimensions. */
  w: number;
  h: number;
  /** Suggested tile width/height on the board. */
  tileW: number;
  tileH: number;
};

function encodeCanvas(
  canvas: HTMLCanvasElement,
  mime: string,
  quality: number,
): string {
  if (mime === "image/png") return canvas.toDataURL("image/png");
  return canvas.toDataURL("image/jpeg", quality);
}

function estimateDataUrlBytes(dataUrl: string): number {
  const comma = dataUrl.indexOf(",");
  if (comma < 0) return dataUrl.length;
  const b64 = dataUrl.slice(comma + 1);
  return Math.ceil((b64.length * 3) / 4);
}

function drawScaled(
  img: HTMLImageElement,
  maxWidth: number,
  mime: string,
  quality: number,
): ImagePayload {
  const scale = Math.min(1, maxWidth / img.width);
  const w = Math.max(1, Math.round(img.width * scale));
  const h = Math.max(1, Math.round(img.height * scale));
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  canvas.getContext("2d")?.drawImage(img, 0, 0, w, h);
  const dataUrl = encodeCanvas(canvas, mime, quality);
  const tileScale = Math.min(1, IMAGE_TILE_MAX_WIDTH / w);
  const tileW = Math.max(120, Math.round(w * tileScale));
  const tileH = Math.max(80, Math.round(h * tileScale));
  return { dataUrl, w, h, tileW, tileH };
}

/**
 * Read an image file for the maw rs / workboard viewer (ported from ssh-share
 * viewer.js). Produces a sharper share encode (up to 1920px, q0.92) plus tile
 * dimensions for the board item.
 */
export function readImageFile(
  file: File,
  onImage: (payload: ImagePayload) => void,
) {
  if (!file || !file.type.startsWith("image/")) return;
  const reader = new FileReader();
  reader.onload = () => {
    const src = reader.result as string;
    const img = new Image();
    img.onload = () => {
      const preferPng = file.type === "image/png";
      let payload = drawScaled(
        img,
        IMAGE_SHARE_MAX_WIDTH,
        preferPng ? "image/png" : "image/jpeg",
        IMAGE_JPEG_QUALITY,
      );
      // If still too large for WS, step down width until it fits or hit floor.
      let maxW = IMAGE_SHARE_MAX_WIDTH;
      while (
        estimateDataUrlBytes(payload.dataUrl) > IMAGE_SHARE_CAP_BYTES &&
        maxW > 480
      ) {
        maxW = Math.round(maxW * 0.75);
        payload = drawScaled(img, maxW, "image/jpeg", IMAGE_JPEG_QUALITY);
      }
      onImage(payload);
    };
    img.onerror = () => {
      onImage({
        dataUrl: src,
        w: 320,
        h: 240,
        tileW: 320,
        tileH: 240,
      });
    };
    img.src = src;
  };
  reader.readAsDataURL(file);
}
