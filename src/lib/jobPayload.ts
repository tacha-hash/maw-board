// Shared "job" board-item payload parser — single source of truth for
// Board.svelte (render) and Session.svelte (mutate/dispatch to bridge).
// This used to be two independently-hand-copied parse functions; that
// exact kind of duplication caused a real bug once already (the lobby
// "Need key" localStorage divergence, PROGRESS.md 2026-07-07 00:05) so
// when Vision Round 3 needed new fields on both sides at once, it made
// sense to collapse them into one module instead of copying a third time.
//
// Field names/values must match tools/board-bridge.ts exactly — it's the
// live consumer, not a spec we control.
export type JobState = "draft" | "pending" | "running" | "done" | "error";
export type MediaType = "image" | "video";

export type JobPayload = {
  prompt: string;
  model: string;
  aspect_ratio: string;
  resolution: string;
  provider: string;
  input_image_ids: string[];
  state: JobState;
  error?: string;
  // Vision Round 3 additions (additive, all optional on the wire) —
  // docs/vision-round3-gen-nodes-design.md
  media_type: MediaType;
  duration?: number;
  negative_prompt?: string;
  end_frame_image_id?: string;
};

const STATES: JobState[] = ["draft", "pending", "running", "done", "error"];
const MEDIA_TYPES: MediaType[] = ["image", "video"];

const DEFAULTS: JobPayload = {
  prompt: "",
  model: "nano-banana",
  aspect_ratio: "1:1",
  resolution: "1K",
  provider: "kie",
  input_image_ids: [],
  state: "draft",
  media_type: "image",
};

export function parseJobPayload(dataUrl: string): JobPayload {
  try {
    const p = JSON.parse(dataUrl);
    return {
      prompt: typeof p.prompt === "string" ? p.prompt : DEFAULTS.prompt,
      model: typeof p.model === "string" ? p.model : DEFAULTS.model,
      aspect_ratio: typeof p.aspect_ratio === "string" ? p.aspect_ratio : DEFAULTS.aspect_ratio,
      resolution: typeof p.resolution === "string" ? p.resolution : DEFAULTS.resolution,
      provider: typeof p.provider === "string" ? p.provider : DEFAULTS.provider,
      input_image_ids: Array.isArray(p.input_image_ids)
        ? p.input_image_ids.filter((x: unknown) => typeof x === "string")
        : [],
      state: STATES.includes(p.state) ? p.state : "draft",
      error: typeof p.error === "string" ? p.error : undefined,
      media_type: MEDIA_TYPES.includes(p.media_type) ? p.media_type : DEFAULTS.media_type,
      duration: typeof p.duration === "number" ? p.duration : undefined,
      negative_prompt: typeof p.negative_prompt === "string" ? p.negative_prompt : undefined,
      end_frame_image_id: typeof p.end_frame_image_id === "string" ? p.end_frame_image_id : undefined,
    };
  } catch {
    return { ...DEFAULTS };
  }
}
