// Gen-job model catalog — shared by the job node's model grid/field
// selectors (Board.svelte). Video entries are restricted to models
// confirmed reachable through Kie today (docs/vision-round3-gen-nodes-
// design.md) — dottodot models marked `kieUnavailable` (Hailuo, Wan, ...)
// need a fal/novita bridge path we don't have yet, so they're left out
// rather than shown and silently failing on Generate.
import type { MediaType } from "./jobPayload";

export type GenModel = {
  id: string;
  name: string;
  brand: string;
  mediaType: MediaType;
  aspectRatios: string[];
  resolutions: string[];
  /** Seconds; omit for models with no duration control (e.g. Veo). */
  durations?: number[];
  /** i2v end-frame reference slot (Kie's imageUrls[1] / input_urls[1]). */
  hasEndFrame?: boolean;
  hasNegativePrompt?: boolean;
};

const IMAGE_ASPECTS = ["1:1", "16:9", "9:16", "4:3", "3:4"];
const IMAGE_RESOLUTIONS = ["1K", "2K", "4K"];

export const GEN_MODELS: GenModel[] = [
  // Must match board-bridge.ts's MODEL_ALIASES keys exactly.
  { id: "nano-banana", name: "nano-banana", brand: "Google", mediaType: "image", aspectRatios: IMAGE_ASPECTS, resolutions: IMAGE_RESOLUTIONS },
  { id: "flux", name: "flux", brand: "Black Forest Labs", mediaType: "image", aspectRatios: IMAGE_ASPECTS, resolutions: IMAGE_RESOLUTIONS },
  { id: "seedream", name: "seedream", brand: "ByteDance", mediaType: "image", aspectRatios: IMAGE_ASPECTS, resolutions: IMAGE_RESOLUTIONS },
  { id: "gpt-image", name: "gpt-image", brand: "OpenAI", mediaType: "image", aspectRatios: IMAGE_ASPECTS, resolutions: IMAGE_RESOLUTIONS },

  // Video — Kie `jobs/createTask` (market) endpoint.
  {
    id: "bytedance/seedance-1.5-pro",
    name: "Seedance 1.5 Pro",
    brand: "ByteDance",
    mediaType: "video",
    aspectRatios: ["16:9", "9:16", "1:1", "4:3", "3:4", "21:9"],
    resolutions: ["480p", "720p"],
    durations: [4, 8, 12],
    hasEndFrame: true,
  },
  // Video — Kie dedicated `/veo/generate` + `/veo/record-info` endpoints
  // (separate path from the market job queue — see design note). No
  // duration param; Veo doesn't accept one.
  {
    id: "veo3.1/fast",
    name: "Veo 3.1 Fast",
    brand: "Google",
    mediaType: "video",
    aspectRatios: ["16:9", "9:16"],
    resolutions: ["720p", "1080p", "4k"],
    hasEndFrame: true,
    hasNegativePrompt: true,
  },
  {
    id: "veo3.1/quality",
    name: "Veo 3.1 Quality",
    brand: "Google",
    mediaType: "video",
    aspectRatios: ["16:9", "9:16"],
    resolutions: ["720p", "1080p", "4k"],
    hasEndFrame: true,
    hasNegativePrompt: true,
  },
];

export function findGenModel(id: string): GenModel | undefined {
  return GEN_MODELS.find((m) => m.id === id);
}

export function modelsFor(mediaType: MediaType): GenModel[] {
  return GEN_MODELS.filter((m) => m.mediaType === mediaType);
}

/** Grouped the same way the old flat MODEL_BRANDS table was, per media type. */
export function brandGroupsFor(mediaType: MediaType): { brand: string; models: GenModel[] }[] {
  const models = modelsFor(mediaType);
  const brands: string[] = [];
  for (const m of models) if (!brands.includes(m.brand)) brands.push(m.brand);
  return brands.map((brand) => ({ brand, models: models.filter((m) => m.brand === brand) }));
}
