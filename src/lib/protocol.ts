type Sid = number; // u32
type Uid = number; // u32

/** Position and size of a window, see the Rust version. */
export type WsWinsize = {
  x: number;
  y: number;
  rows: number;
  cols: number;
};

/** Information about a user, see the Rust version */
export type WsUser = {
  name: string;
  cursor: [number, number] | null;
  focus: number | null;
  canWrite: boolean;
};

/** A collaborative board item — maw share extension. For kind:"stream" this is a
 *  placeholder tile (no frame data); live frames arrive via StreamFrame. */
export type BoardItem = {
  id: string;
  kind: string; // "image" | "stream"
  x: number;
  y: number;
  w: number;
  h: number;
  dataUrl: string; // image data; empty for stream placeholders
};

/** Server message type, see the Rust version. */
export type WsServer = {
  hello?: [Uid, string];
  invalidAuth?: [];
  users?: [Uid, WsUser][];
  userDiff?: [Uid, WsUser | null];
  shells?: [Sid, WsWinsize][];
  chunks?: [Sid, number, Uint8Array[]];
  hear?: [Uid, string, string];
  shellLatency?: number | bigint;
  pong?: number | bigint;
  // ── maw share workboard extensions ──
  voiceData?: [Uid, Uint8Array];
  streamFrame?: [Uid, string, Uint8Array];
  board?: BoardItem[];
  boardPut?: BoardItem;
  boardMove?: [string, number, number];
  boardDelete?: string;
  // ── WebRTC signaling relay ──
  signal?: [Uid, string]; // [from_uid, payload_json]
  error?: string;
};

/** Client message type, see the Rust version. */
export type WsClient = {
  authenticate?: [Uint8Array, Uint8Array | null];
  setName?: string;
  setCursor?: [number, number] | null;
  setFocus?: number | null;
  create?: [number, number];
  close?: Sid;
  move?: [Sid, WsWinsize | null];
  data?: [Sid, Uint8Array, bigint];
  subscribe?: [Sid, number];
  chat?: string;
  ping?: bigint;
  // ── maw share workboard extensions ──
  voice?: Uint8Array;
  streamFrame?: [string, Uint8Array];
  boardPut?: BoardItem;
  boardMove?: [string, number, number];
  boardDelete?: string;
  // ── WebRTC signaling relay ──
  signal?: [number, string]; // [target_uid, payload_json]
};
