// Shared "order"/"order-status" board-item payload parsers — Vision Round 4
// item 4 (Work Order node), docs/vision-round4-work-order-design.md.
//
// Split into two items on purpose (not one, like "job"): job's
// collaborative-write pattern is safe because the two sides write at
// different *times* (generation takes seconds) — a boardPut always replaces
// the whole dataUrl blob, not a per-key merge, so two sides racing to write
// at genuinely the same moment would silently drop whichever wrote first.
// A Work Order runs for minutes/hours, so a prompt edit and an incoming
// result can really collide. kind:"order" is frontend-owned
// (title/prompt/inputs/dispatch_seq); kind:"order-status" (bridge-owned:
// last_dispatch_seq/status/results) is joined at render time, the same way
// job already joins with its <id>-out sibling.
//
// Field names/values must match tools/board-bridge.ts exactly — it's the
// live consumer, not a spec we control.

export type OrderPayload = {
  title: string;
  prompt: string;
  inputs: string[];
  dispatch_seq: number;
};

const ORDER_DEFAULTS: OrderPayload = {
  title: "",
  prompt: "",
  inputs: [],
  dispatch_seq: 0,
};

export function parseOrderPayload(dataUrl: string): OrderPayload {
  try {
    const p = JSON.parse(dataUrl);
    return {
      title: typeof p.title === "string" ? p.title : ORDER_DEFAULTS.title,
      prompt: typeof p.prompt === "string" ? p.prompt : ORDER_DEFAULTS.prompt,
      inputs: Array.isArray(p.inputs) ? p.inputs.filter((x: unknown) => typeof x === "string") : [],
      dispatch_seq: typeof p.dispatch_seq === "number" ? p.dispatch_seq : ORDER_DEFAULTS.dispatch_seq,
    };
  } catch {
    return { ...ORDER_DEFAULTS };
  }
}

export type OrderAssigneeStatus = "pending" | "working" | "done" | "error";
export type OrderResult = { agent: string; itemId: string; ts: number };

export type OrderStatusPayload = {
  last_dispatch_seq: number;
  status: Record<string, OrderAssigneeStatus>;
  results: OrderResult[];
};

const STATUSES: OrderAssigneeStatus[] = ["pending", "working", "done", "error"];

const STATUS_DEFAULTS: OrderStatusPayload = {
  last_dispatch_seq: 0,
  status: {},
  results: [],
};

export function orderStatusItemId(orderId: string): string {
  return `order-status:${orderId}`;
}

export function parseOrderStatusPayload(dataUrl: string): OrderStatusPayload {
  try {
    const p = JSON.parse(dataUrl);
    const status: Record<string, OrderAssigneeStatus> = {};
    if (p.status && typeof p.status === "object") {
      for (const [agent, s] of Object.entries(p.status)) {
        if (typeof agent === "string" && STATUSES.includes(s as OrderAssigneeStatus)) {
          status[agent] = s as OrderAssigneeStatus;
        }
      }
    }
    const results: OrderResult[] = Array.isArray(p.results)
      ? p.results.filter(
          (r: unknown): r is OrderResult =>
            !!r && typeof r === "object" &&
            typeof (r as any).agent === "string" &&
            typeof (r as any).itemId === "string" &&
            typeof (r as any).ts === "number",
        )
      : [];
    return {
      last_dispatch_seq: typeof p.last_dispatch_seq === "number" ? p.last_dispatch_seq : STATUS_DEFAULTS.last_dispatch_seq,
      status,
      results,
    };
  } catch {
    return { last_dispatch_seq: 0, status: {}, results: [] };
  }
}

/** Latest result entry for a given agent (an agent can submit more than
 * once — table shows the most recent, full history stays in `results`). */
export function latestResultFor(results: OrderResult[], agent: string): OrderResult | undefined {
  let latest: OrderResult | undefined;
  for (const r of results) {
    if (r.agent === agent && (!latest || r.ts > latest.ts)) latest = r;
  }
  return latest;
}
