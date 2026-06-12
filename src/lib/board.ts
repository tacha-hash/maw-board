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

/**
 * Read an image file, downscale it (max 720px wide, JPEG q0.82), and deliver
 * the resulting data URL and dimensions for a `{ kind: "image" }` board item.
 */
export function readImageFile(
  file: File,
  onImage: (dataUrl: string, w: number, h: number) => void,
) {
  if (!file || !file.type.startsWith("image/")) return;
  const reader = new FileReader();
  reader.onload = () => {
    const src = reader.result as string;
    const img = new Image();
    img.onload = () => {
      const scale = Math.min(1, 720 / img.width);
      const w = Math.round(img.width * scale);
      const h = Math.round(img.height * scale);
      const canvas = document.createElement("canvas");
      canvas.width = w;
      canvas.height = h;
      canvas.getContext("2d")?.drawImage(img, 0, 0, w, h);
      onImage(canvas.toDataURL("image/jpeg", 0.82), w, h);
    };
    img.onerror = () => onImage(src, 320, 240);
    img.src = src;
  };
  reader.readAsDataURL(file);
}
