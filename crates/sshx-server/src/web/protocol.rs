//! Serializable types sent and received by the web server.

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use sshx_core::{Sid, Uid};

/// Real-time message conveying the position and size of a terminal.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WsWinsize {
    /// The top-left x-coordinate of the window, offset from origin.
    pub x: i32,
    /// The top-left y-coordinate of the window, offset from origin.
    pub y: i32,
    /// The number of rows in the window.
    pub rows: u16,
    /// The number of columns in the terminal.
    pub cols: u16,
    /// Which backend (physical node) this shell runs on. Wired through now
    /// so a future per-backend badge/color UI doesn't need another protocol
    /// change — frontends that don't use it yet can just ignore the field.
    #[serde(default)]
    pub backend_id: u32,
}

impl Default for WsWinsize {
    fn default() -> Self {
        WsWinsize {
            x: 0,
            y: 0,
            // Taller default so new terminals open portrait-ish (Bo directive
            // 2026-06-13: "default terminal วินโดวส์ แนวตั้งใหญ่ขึ้น"). Width
            // unchanged at 80; height bumped 24 -> 40. Per-terminal ▯/▭ presets
            // still let users override (XTerm.svelte).
            rows: 40,
            cols: 80,
            backend_id: 0,
        }
    }
}

/// Real-time message providing information about a user.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WsUser {
    /// The user's display name.
    pub name: String,
    /// Live coordinates of the mouse cursor, if available.
    pub cursor: Option<(i32, i32)>,
    /// Currently focused terminal window ID.
    pub focus: Option<Sid>,
    /// Whether the user has write permissions in the session.
    pub can_write: bool,
}

/// A collaborative board item — a maw share workboard extension on top of sshx's
/// terminals. Persisted in the session snapshot. For `kind:"stream"` this is just
/// a placeholder tile (no frame data); live frames flow ephemerally via
/// `StreamFrame` so the snapshot stays small and the broadcast channel can't flood.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BoardItem {
    /// Stable item id (client-generated).
    pub id: String,
    /// Item kind: "image" or "stream".
    pub kind: String,
    /// World-space x position.
    pub x: i32,
    /// World-space y position.
    pub y: i32,
    /// Render width in pixels.
    pub w: u32,
    /// Render height in pixels.
    pub h: u32,
    /// Data URL of the image (empty for `kind:"stream"` placeholders).
    pub data_url: String,
}

/// A real-time message sent from the server over WebSocket.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub enum WsServer {
    /// Initial server message, with the user's ID and session metadata.
    Hello(Uid, String),
    /// The user's authentication was invalid.
    InvalidAuth(),
    /// A snapshot of all current users in the session.
    Users(Vec<(Uid, WsUser)>),
    /// Info about a single user in the session: joined, left, or changed.
    UserDiff(Uid, Option<WsUser>),
    /// Notification when the set of open shells has changed.
    Shells(Vec<(Sid, WsWinsize)>),
    /// Subscription results, in the form of terminal data chunks.
    Chunks(Sid, u64, Vec<Bytes>),
    /// Get a chat message tuple `(uid, name, text)` from the room.
    Hear(Uid, String, String),
    /// Forward a latency measurement between the server and backend shell.
    ShellLatency(u64),
    /// Echo back a timestamp, for the the client's own latency measurement.
    Pong(u64),
    // ── maw share workboard extensions ──
    /// Broadcast a push-to-talk voice clip from a user (opus/webm bytes).
    /// Ephemeral — relayed, never persisted.
    VoiceData(Uid, Bytes),
    /// Broadcast one live screen-share frame `(uid, stream_id, jpeg_bytes)`.
    /// Ephemeral — relayed, never persisted (keeps the snapshot small).
    StreamFrame(Uid, String, Bytes),
    /// Full board snapshot (images + stream placeholder tiles), sent on join.
    Board(Vec<BoardItem>),
    /// A board item was added or updated (image add, stream frame).
    BoardPut(BoardItem),
    /// A board item moved to a new position: `(id, x, y)`.
    BoardMove(String, i32, i32),
    /// A board item was removed: `id`.
    BoardDelete(String),
    /// WebRTC signaling: `(from_uid, to_uid, sdp/ice payload)`.
    /// Broadcast to all; client filters by `to == self`. Ephemeral.
    Signal(Uid, Uid, String),
    /// Alert the client of an application error.
    Error(String),
}

/// A real-time message sent from the client over WebSocket.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub enum WsClient {
    /// Authenticate the user's encryption key by zeros block and write password
    /// (if provided).
    Authenticate(Bytes, Option<Bytes>),
    /// Set the name of the current user.
    SetName(String),
    /// Send real-time information about the user's cursor.
    SetCursor(Option<(i32, i32)>),
    /// Set the currently focused shell.
    SetFocus(Option<Sid>),
    /// Create a new shell.
    Create(i32, i32),
    /// Close a specific shell.
    Close(Sid),
    /// Move a shell window to a new position and focus it.
    Move(Sid, Option<WsWinsize>),
    /// Add user data to a given shell.
    Data(Sid, Bytes, u64),
    /// Subscribe to a shell, starting at a given chunk index.
    Subscribe(Sid, u64),
    /// Send a a chat message to the room.
    Chat(String),
    /// Send a ping to the server, for latency measurement.
    Ping(u64),
    // ── maw share workboard extensions ──
    /// Push-to-talk: send a voice clip (opus/webm bytes) to relay to the room.
    Voice(Bytes),
    /// Send one live screen-share frame `(stream_id, jpeg_bytes)` to relay
    /// (ephemeral, client throttles to ~3 fps).
    StreamFrame(String, Bytes),
    /// Add or update a board item (image add, or a stream placeholder tile).
    BoardPut(BoardItem),
    /// Move a board item to a new position: `(id, x, y)`.
    BoardMove(String, i32, i32),
    /// Remove a board item: `id`.
    BoardDelete(String),
    /// WebRTC signaling: `(target_uid, sdp/ice payload)`. Server broadcasts
    /// as `WsServer::Signal(from, to, payload)`; client filters `to == self`.
    Signal(Uid, String),
}
