//! Core logic for sshx sessions, independent of message transport.

use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use parking_lot::{Mutex, RwLock, RwLockWriteGuard};
use sshx_core::{
    proto::{server_update::ServerMessage, SequenceNumbers},
    BackendId, IdCounter, Sid, Uid,
};
use tokio::sync::{broadcast, watch, Notify};
use tokio::time::Instant;
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream, WatchStream};
use tokio_stream::Stream;
use tracing::{debug, warn};

use crate::utils::Shutdown;
use crate::web::protocol::{BoardItem, WsServer, WsUser, WsWinsize};

mod snapshot;

/// Store a rolling buffer with at most this quantity of output, per shell.
const SHELL_STORED_BYTES: u64 = 1 << 21; // 2 MiB

/// Maximum collaborative board items per session.
pub(crate) const MAX_BOARD_ITEMS: usize = 128;
/// Maximum bytes per board item `data_url`.
const MAX_BOARD_DATA_URL_BYTES: usize = 2 << 20; // 2 MiB
const MAX_BOARD_ID_LEN: usize = 256;
const MAX_BOARD_KIND_LEN: usize = 64;
const MIN_BOARD_DIM: u32 = 1;
const MAX_BOARD_DIM: u32 = 16_384;

pub(crate) fn validate_board_item(item: &BoardItem) -> Result<()> {
    use anyhow::ensure;
    ensure!(!item.id.is_empty(), "board item id empty");
    ensure!(
        item.id.len() <= MAX_BOARD_ID_LEN,
        "board item id too long (max {MAX_BOARD_ID_LEN})"
    );
    ensure!(
        item.kind.len() <= MAX_BOARD_KIND_LEN,
        "board item kind too long (max {MAX_BOARD_KIND_LEN})"
    );
    ensure!(
        item.w >= MIN_BOARD_DIM && item.h >= MIN_BOARD_DIM,
        "board item dimensions too small (min {MIN_BOARD_DIM})"
    );
    ensure!(
        item.w <= MAX_BOARD_DIM && item.h <= MAX_BOARD_DIM,
        "board item dimensions too large (max {MAX_BOARD_DIM})"
    );
    ensure!(
        item.data_url.len() <= MAX_BOARD_DATA_URL_BYTES,
        "board item data_url too large (max {MAX_BOARD_DATA_URL_BYTES} bytes)"
    );
    Ok(())
}

/// Static metadata for this session.
#[derive(Debug, Clone)]
pub struct Metadata {
    /// Used to validate that clients have the correct encryption key.
    pub encrypted_zeros: Bytes,

    /// Name of the session (human-readable).
    pub name: String,

    /// Password for write access to the session.
    pub write_password_hash: Option<Bytes>,
}

/// Per-backend channel state — one of these exists per registered `BackendId`,
/// for the lifetime of the session (not just while connected), so that
/// messages queued while a backend is briefly disconnected are delivered once
/// it reconnects, matching the pre-multi-backend reconnect semantics.
#[derive(Debug)]
struct BackendChannel {
    /// Human-readable label (hostname, etc.), from Open()'s implicit
    /// "primary" registration or Join()'s `backend_name`.
    name: String,

    /// Whether a `Channel()` stream currently holds this backend's receiver.
    /// Prevents a second concurrent connection for the same backend_id from
    /// racing on the same receiver (see docs/phase3-design.md's MPMC finding).
    connected: Mutex<bool>,

    /// Sender end of a channel that buffers messages for this backend.
    update_tx: async_channel::Sender<ServerMessage>,

    /// Receiver end of a channel that buffers messages for this backend.
    update_rx: async_channel::Receiver<ServerMessage>,
}

/// In-memory state for a single sshx session.
#[derive(Debug)]
pub struct Session {
    /// Static metadata for this session.
    metadata: Metadata,

    /// In-memory state for the session.
    shells: RwLock<HashMap<Sid, State>>,

    /// Metadata for currently connected users.
    users: RwLock<HashMap<Uid, WsUser>>,

    /// Atomic counter to get new, unique IDs.
    counter: IdCounter,

    /// Registered backends (the "primary" from Open(), plus any from Join()),
    /// each with its own message channel. Entries persist for the life of the
    /// session, not just while a gRPC stream is attached.
    backends: RwLock<HashMap<BackendId, BackendChannel>>,

    /// Round-robin cursor for `pick_backend_for_create`.
    next_create_backend: AtomicU32,

    /// Timestamp of the last backend client message from an active connection.
    last_accessed: Mutex<Instant>,

    /// Watch channel source for the ordered list of open shells and sizes.
    source: watch::Sender<Vec<(Sid, WsWinsize)>>,

    /// Broadcasts updates to all WebSocket clients.
    ///
    /// Every update inside this channel must be of idempotent form, since
    /// messages may arrive before or after any snapshot of the current session
    /// state. Duplicated events should remain consistent.
    broadcast: broadcast::Sender<WsServer>,

    /// Triggered from metadata events when an immediate snapshot is needed.
    sync_notify: Notify,

    /// Collaborative board items — in-memory, for late-join snapshot replay.
    board: Mutex<Vec<BoardItem>>,

    /// Set when this session has been closed and removed.
    shutdown: Shutdown,
}

/// Internal state for each shell.
#[derive(Debug)]
struct State {
    /// Sequence number, indicating how many bytes have been received.
    seqnum: u64,

    /// Terminal data chunks.
    data: Vec<Bytes>,

    /// Number of pruned data chunks before `data[0]`.
    chunk_offset: u64,

    /// Number of bytes in pruned data chunks.
    byte_offset: u64,

    /// Set when this shell is terminated.
    closed: bool,

    /// Updated when any of the above fields change.
    notify: Arc<Notify>,

    /// Which backend owns this shell — Input/Resize/Close route here.
    backend_id: BackendId,
}

impl Default for State {
    fn default() -> Self {
        Self {
            seqnum: 0,
            data: Vec::new(),
            chunk_offset: 0,
            byte_offset: 0,
            closed: false,
            notify: Arc::default(),
            backend_id: BackendId::PRIMARY,
        }
    }
}

impl Session {
    /// Construct a new session. Does NOT register any backend — callers must
    /// call `register_backend()` (for a fresh `Open()`) or `restore_backend()`
    /// (once per backend, when restoring from a snapshot) afterwards.
    pub fn new(metadata: Metadata) -> Self {
        let now = Instant::now();
        Session {
            metadata,
            shells: RwLock::new(HashMap::new()),
            users: RwLock::new(HashMap::new()),
            counter: IdCounter::default(),
            backends: RwLock::new(HashMap::new()),
            next_create_backend: AtomicU32::new(0),
            last_accessed: Mutex::new(now),
            source: watch::channel(Vec::new()).0,
            broadcast: broadcast::channel(64).0,
            sync_notify: Notify::new(),
            board: Mutex::new(Vec::new()),
            shutdown: Shutdown::new(),
        }
    }

    /// Returns the metadata for this session.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Register a new backend (via `Open()` for the primary, or `Join()` for
    /// any other), allocating a fresh `BackendId` and message channel.
    pub fn register_backend(&self, name: String) -> BackendId {
        let id = self.counter.next_backend_id();
        self.restore_backend(id, name);
        id
    }

    /// Register a backend under a SPECIFIC, already-allocated ID — used only
    /// when restoring from a snapshot, where backend identities must be
    /// preserved exactly so already-connected clients can still reconnect.
    pub fn restore_backend(&self, id: BackendId, name: String) {
        let (update_tx, update_rx) = async_channel::bounded(256);
        self.backends.write().insert(
            id,
            BackendChannel {
                name,
                connected: Mutex::new(false),
                update_tx,
                update_rx,
            },
        );
    }

    /// List all registered backends (id, name) — used for the snapshot and
    /// could back a future "which node is this terminal on" UI.
    pub fn list_backends(&self) -> Vec<(BackendId, String)> {
        self.backends
            .read()
            .iter()
            .map(|(id, b)| (*id, b.name.clone()))
            .collect()
    }

    /// Attach to a registered backend's channel, for exclusive use by one
    /// `Channel()` stream at a time. Fails if the backend doesn't exist, or
    /// already has an active connection (see `BackendChannel::connected`) —
    /// the latter is what prevents the MPMC race a second concurrent
    /// connection to the same backend would otherwise cause.
    pub fn connect_backend(&self, id: BackendId) -> Result<async_channel::Receiver<ServerMessage>> {
        let backends = self.backends.read();
        let backend = backends
            .get(&id)
            .with_context(|| format!("backend {id} not registered"))?;
        let mut connected = backend.connected.lock();
        if *connected {
            bail!("backend {id} already has an active connection");
        }
        *connected = true;
        Ok(backend.update_rx.clone())
    }

    /// Release a backend's connection, allowing a future reconnect. Does NOT
    /// remove the backend or affect its shells — messages sent to it in the
    /// meantime simply queue in its channel, same as the original
    /// single-backend reconnect behavior.
    pub fn disconnect_backend(&self, id: BackendId) {
        if let Some(backend) = self.backends.read().get(&id) {
            *backend.connected.lock() = false;
        }
    }

    /// Get the sender for a specific backend's channel, for routing messages
    /// (Input/Resize/Close) from the browser to the backend that owns a
    /// given shell. Works regardless of current connection state.
    pub fn backend_sender(&self, id: BackendId) -> Option<async_channel::Sender<ServerMessage>> {
        self.backends.read().get(&id).map(|b| b.update_tx.clone())
    }

    /// Pick a backend to receive a newly-created shell when the browser
    /// doesn't specify one — round-robins over all currently registered
    /// backends. A real backend-selector UI can replace this later without a
    /// protocol change (see phase3-design.md).
    pub fn pick_backend_for_create(&self) -> Option<BackendId> {
        let backends = self.backends.read();
        let mut ids: Vec<BackendId> = backends.keys().copied().collect();
        if ids.is_empty() {
            return None;
        }
        ids.sort();
        let idx = self.next_create_backend.fetch_add(1, Ordering::Relaxed) as usize % ids.len();
        Some(ids[idx])
    }

    /// Look up which backend owns a given shell.
    pub fn shell_backend(&self, id: Sid) -> Option<BackendId> {
        self.shells.read().get(&id).map(|s| s.backend_id)
    }

    /// Gives access to the ID counter for obtaining new IDs.
    pub fn counter(&self) -> &IdCounter {
        &self.counter
    }

    /// Return the sequence numbers for current shells.
    /// Sequence numbers for shells owned by ONE specific backend. Each
    /// backend's `Channel()` connection must only ever be sent sync info for
    /// its OWN shells: a backend that sees a Sid it doesn't recognize
    /// responds by telling the server to close it (a stale-state cleanup
    /// mechanism from the single-backend design) — sending it every
    /// backend's shells indiscriminately would have every OTHER backend's
    /// shells reported as unknown and closed within seconds of creation.
    pub fn sequence_numbers(&self, backend_id: BackendId) -> SequenceNumbers {
        let shells = self.shells.read();
        let mut map = HashMap::new();
        for (key, value) in &*shells {
            if !value.closed && value.backend_id == backend_id {
                map.insert(key.0, value.seqnum);
            }
        }
        SequenceNumbers { map }
    }

    /// Receive a notification on broadcasted message events.
    pub fn subscribe_broadcast(
        &self,
    ) -> impl Stream<Item = Result<WsServer, BroadcastStreamRecvError>> + Unpin {
        BroadcastStream::new(self.broadcast.subscribe())
    }

    /// Receive a notification every time the set of shells is changed.
    pub fn subscribe_shells(&self) -> impl Stream<Item = Vec<(Sid, WsWinsize)>> + Unpin {
        WatchStream::new(self.source.subscribe())
    }

    /// Subscribe for chunks from a shell, until it is closed.
    pub fn subscribe_chunks(
        &self,
        id: Sid,
        mut chunknum: u64,
    ) -> impl Stream<Item = (u64, Vec<Bytes>)> + '_ {
        async_stream::stream! {
            while !self.shutdown.is_terminated() {
                // We absolutely cannot hold `shells` across an await point,
                // since that would cause deadlocks.
                let (seqnum, chunks, notified) = {
                    let shells = self.shells.read();
                    let shell = match shells.get(&id) {
                        Some(shell) if !shell.closed => shell,
                        _ => return,
                    };
                    let notify = Arc::clone(&shell.notify);
                    let notified = async move { notify.notified().await };
                    let mut seqnum = shell.byte_offset;
                    let mut chunks = Vec::new();
                    let current_chunks = shell.chunk_offset + shell.data.len() as u64;
                    if chunknum < current_chunks {
                        let start = chunknum.saturating_sub(shell.chunk_offset) as usize;
                        seqnum += shell.data[..start].iter().map(|x| x.len() as u64).sum::<u64>();
                        chunks = shell.data[start..].to_vec();
                        chunknum = current_chunks;
                    }
                    (seqnum, chunks, notified)
                };

                if !chunks.is_empty() {
                    yield (seqnum, chunks);
                }
                tokio::select! {
                    _ = notified => (),
                    _ = self.terminated() => return,
                }
            }
        }
    }

    /// Add a new shell to the session, owned by the given backend.
    pub fn add_shell(&self, id: Sid, center: (i32, i32), backend_id: BackendId) -> Result<()> {
        use std::collections::hash_map::Entry::*;
        let _guard = match self.shells.write().entry(id) {
            Occupied(_) => bail!("shell already exists with id={id}"),
            Vacant(v) => v.insert(State {
                backend_id,
                ..Default::default()
            }),
        };
        self.source.send_modify(|source| {
            let winsize = WsWinsize {
                x: center.0,
                y: center.1,
                backend_id: backend_id.0,
                ..Default::default()
            };
            source.push((id, winsize));
        });
        self.sync_now();
        Ok(())
    }

    /// Terminates an existing shell.
    pub fn close_shell(&self, id: Sid) -> Result<()> {
        match self.shells.write().get_mut(&id) {
            Some(shell) if !shell.closed => {
                shell.closed = true;
                shell.notify.notify_waiters();
            }
            Some(_) => return Ok(()),
            None => bail!("cannot close shell with id={id}, does not exist"),
        }
        self.source.send_modify(|source| {
            source.retain(|&(x, _)| x != id);
        });
        self.sync_now();
        Ok(())
    }

    fn get_shell_mut(&self, id: Sid) -> Result<impl DerefMut<Target = State> + '_> {
        let shells = self.shells.write();
        match shells.get(&id) {
            Some(shell) if !shell.closed => {
                Ok(RwLockWriteGuard::map(shells, |s| s.get_mut(&id).unwrap()))
            }
            Some(_) => bail!("cannot update shell with id={id}, already closed"),
            None => bail!("cannot update shell with id={id}, does not exist"),
        }
    }

    /// Change the size of a terminal, notifying clients if necessary.
    pub fn move_shell(&self, id: Sid, winsize: Option<WsWinsize>) -> Result<()> {
        let _guard = self.get_shell_mut(id)?; // Ensures mutual exclusion.
        self.source.send_modify(|source| {
            if let Some(idx) = source.iter().position(|&(sid, _)| sid == id) {
                let (_, oldsize) = source.remove(idx);
                source.push((id, winsize.unwrap_or(oldsize)));
            }
        });
        Ok(())
    }

    /// Receive new data into the session.
    pub fn add_data(&self, id: Sid, data: Bytes, seq: u64) -> Result<()> {
        let mut shell = self.get_shell_mut(id)?;

        if seq <= shell.seqnum && seq + data.len() as u64 > shell.seqnum {
            let start = shell.seqnum - seq;
            let segment = data.slice(start as usize..);
            debug!(%id, bytes = segment.len(), "adding data to shell");
            shell.seqnum += segment.len() as u64;
            shell.data.push(segment);

            // Prune old chunks if we've exceeded the maximum stored bytes.
            let mut stored_bytes = shell.seqnum - shell.byte_offset;
            if stored_bytes > SHELL_STORED_BYTES {
                let mut offset = 0;
                while offset < shell.data.len() && stored_bytes > SHELL_STORED_BYTES {
                    let bytes = shell.data[offset].len() as u64;
                    stored_bytes -= bytes;
                    shell.chunk_offset += 1;
                    shell.byte_offset += bytes;
                    offset += 1;
                }
                shell.data.drain(..offset);
            }

            shell.notify.notify_waiters();
        }

        Ok(())
    }

    /// List all the users in the session.
    pub fn list_users(&self) -> Vec<(Uid, WsUser)> {
        self.users
            .read()
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect()
    }

    /// Update a user in place by ID, applying a callback to the object.
    pub fn update_user(&self, id: Uid, f: impl FnOnce(&mut WsUser)) -> Result<()> {
        let updated_user = {
            let mut users = self.users.write();
            let user = users.get_mut(&id).context("user not found")?;
            f(user);
            user.clone()
        };
        self.broadcast
            .send(WsServer::UserDiff(id, Some(updated_user)))
            .ok();
        Ok(())
    }

    /// Add a new user, and return a guard that removes the user when dropped.
    pub fn user_scope(&self, id: Uid, can_write: bool) -> Result<impl Drop + '_> {
        use std::collections::hash_map::Entry::*;

        #[must_use]
        struct UserGuard<'a>(&'a Session, Uid);
        impl Drop for UserGuard<'_> {
            fn drop(&mut self) {
                self.0.remove_user(self.1);
            }
        }

        match self.users.write().entry(id) {
            Occupied(_) => bail!("user already exists with id={id}"),
            Vacant(v) => {
                let user = WsUser {
                    name: format!("User {id}"),
                    cursor: None,
                    focus: None,
                    can_write,
                };
                v.insert(user.clone());
                self.broadcast.send(WsServer::UserDiff(id, Some(user))).ok();
                Ok(UserGuard(self, id))
            }
        }
    }

    /// Remove an existing user.
    fn remove_user(&self, id: Uid) {
        if self.users.write().remove(&id).is_none() {
            warn!(%id, "invariant violation: removed user that does not exist");
        }
        self.broadcast.send(WsServer::UserDiff(id, None)).ok();
    }

    /// Check if a user has write permission in the session.
    pub fn check_write_permission(&self, user_id: Uid) -> Result<()> {
        let users = self.users.read();
        let user = users.get(&user_id).context("user not found")?;
        if !user.can_write {
            bail!("No write permission");
        }
        Ok(())
    }

    /// Send a chat message into the room.
    pub fn send_chat(&self, id: Uid, msg: &str) -> Result<()> {
        // Populate the message with the current name in case it's not known later.
        let name = {
            let users = self.users.read();
            users.get(&id).context("user not found")?.name.clone()
        };
        self.broadcast
            .send(WsServer::Hear(id, name, msg.into()))
            .ok();
        Ok(())
    }

    /// Return a snapshot of all current board items (sent to newly-joined clients).
    pub fn board_snapshot(&self) -> Vec<BoardItem> {
        self.board.lock().clone()
    }

    /// Add or update a board item, broadcasting to all clients.
    pub fn board_put(&self, item: BoardItem) -> Result<()> {
        validate_board_item(&item)?;
        {
            let mut board = self.board.lock();
            if let Some(existing) = board.iter_mut().find(|b| b.id == item.id) {
                *existing = item.clone();
            } else {
                if board.len() >= MAX_BOARD_ITEMS {
                    bail!("board item limit reached (max {MAX_BOARD_ITEMS})");
                }
                board.push(item.clone());
            }
        }
        self.broadcast.send(WsServer::BoardPut(item)).ok();
        Ok(())
    }

    /// Move a board item to a new position, broadcasting to all clients.
    pub fn board_move(&self, id: &str, x: i32, y: i32) {
        {
            let mut board = self.board.lock();
            if let Some(item) = board.iter_mut().find(|b| b.id == id) {
                item.x = x;
                item.y = y;
            }
        }
        self.broadcast
            .send(WsServer::BoardMove(id.to_owned(), x, y))
            .ok();
    }

    /// Remove a board item, broadcasting to all clients.
    pub fn board_delete(&self, id: &str) {
        self.board.lock().retain(|b| b.id != id);
        self.broadcast
            .send(WsServer::BoardDelete(id.to_owned()))
            .ok();
    }

    /// Broadcast a voice clip to all clients (ephemeral — not persisted).
    pub fn send_voice(&self, uid: Uid, data: Bytes) {
        self.broadcast.send(WsServer::VoiceData(uid, data)).ok();
    }

    /// Broadcast a screen-share frame to all clients (ephemeral — not persisted).
    pub fn send_stream_frame(&self, uid: Uid, stream_id: String, data: Bytes) {
        self.broadcast
            .send(WsServer::StreamFrame(uid, stream_id, data))
            .ok();
    }

    /// Broadcast a WebRTC signaling message (ephemeral — not persisted).
    /// All clients receive it; the intended recipient filters by `to == self`.
    pub fn send_signal(&self, from: Uid, to: Uid, payload: String) {
        self.broadcast
            .send(WsServer::Signal(from, to, payload))
            .ok();
    }

    /// Send a measurement of the shell latency.
    pub fn send_latency_measurement(&self, latency: u64) {
        self.broadcast.send(WsServer::ShellLatency(latency)).ok();
    }

    /// Register a backend client heartbeat, refreshing the timestamp.
    pub fn access(&self) {
        *self.last_accessed.lock() = Instant::now();
    }

    /// Returns the timestamp of the last backend client activity.
    pub fn last_accessed(&self) -> Instant {
        *self.last_accessed.lock()
    }


    /// Mark the session as requiring an immediate storage sync.
    ///
    /// This is needed for consistency when creating new shells, removing old
    /// shells, or updating the ID counter. If these operations are lost in a
    /// server restart, then the snapshot that contains them would be invalid
    /// compared to the current backend client state.
    ///
    /// Note that it is not necessary to do this all the time though, since that
    /// would put too much pressure on the database. Lost terminal data is
    /// already re-synchronized periodically.
    pub fn sync_now(&self) {
        self.sync_notify.notify_one();
    }

    /// Resolves when the session has been marked for an immediate sync.
    pub async fn sync_now_wait(&self) {
        self.sync_notify.notified().await
    }

    /// Send a termination signal to exit this session.
    pub fn shutdown(&self) {
        self.shutdown.shutdown()
    }

    /// Resolves when the session has received a shutdown signal.
    pub async fn terminated(&self) {
        self.shutdown.wait().await
    }
}
