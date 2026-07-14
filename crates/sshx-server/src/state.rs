//! Stateful components of the server, managing multiple sessions.

use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use dashmap::DashMap;
use hmac::{Hmac, Mac as _};
use sha2::Sha256;
use sshx_core::{rand_alphanumeric, BackendId};
use tokio::sync::mpsc;
use tokio::time;
use tokio_stream::StreamExt;
use tracing::error;

use self::accounts::AccountsDb;
use self::disk::StorageDisk;
use self::mesh::StorageMesh;
use crate::session::Session;
use crate::ServerOptions;

pub mod accounts;
pub mod disk;
pub mod mesh;

/// Timeout for a disconnected session to be evicted and closed.
///
/// If a session has no backend clients making connections in this interval,
/// then its updated timestamp will be out-of-date, so we close it and remove it
/// from the state to reduce memory usage.
const DISCONNECTED_SESSION_EXPIRY: Duration = Duration::from_secs(300);

/// A control-channel frame pushed from the server to a connected connector
/// (Vision Round 5 F1a). The connector consumes these instead of polling
/// `/api/boards`, so a new/shared board is served in ~0s instead of one poll
/// interval, and the server knows in real time which connectors are online.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ConnectorEvent {
    /// The full set of board names this account should serve, sent once right
    /// after connect. The connector reconciles against it: join boards it isn't
    /// serving yet, drop any it's serving that vanished.
    Snapshot {
        /// The connector's own account id — so it can tag the agent rosters it
        /// posts as `roster:<account_id>`, keeping each account's agents visible
        /// only to that account (no cross-account leak on shared boards).
        account_id: String,
        /// Every board name the account may serve at connect time.
        boards: Vec<String>,
    },
    /// Begin serving one board (it was just created by, or shared to, this
    /// account).
    Serve {
        /// The board name to start serving.
        board: String,
    },
    /// Stop serving one board (deleted, or unshared from this account).
    Unserve {
        /// The board name to stop serving.
        board: String,
    },
    /// Heartbeat, so an idle channel isn't mistaken for dead by intermediaries.
    Ping,
}

/// One live connector control channel: an id (to unregister the exact
/// connection on disconnect) and the sender half of its outbound event queue.
struct ConnectorHandle {
    id: u64,
    tx: mpsc::UnboundedSender<ConnectorEvent>,
}

/// All live connectors for a single account. A user may run the connector on
/// several machines at once, so this is a list, not a single handle.
#[derive(Default)]
struct ConnectorRegistry {
    handles: Vec<ConnectorHandle>,
    next_id: u64,
}

/// Shared state object for global server logic.
pub struct ServerState {
    /// Message authentication code for signing tokens.
    mac: Hmac<Sha256>,

    /// Override the origin returned for the Open() RPC.
    override_origin: Option<String>,

    /// Allowed browser Origin for cross-origin-sensitive requests, if set.
    allowed_origin: Option<String>,

    /// Whether to emit session cookies without the `Secure` attribute (dev).
    insecure_cookies: bool,

    /// A concurrent map of session IDs to session objects.
    store: DashMap<String, Arc<Session>>,

    /// Storage and distributed communication provider, if enabled.
    mesh: Option<StorageMesh>,

    /// Durable on-disk session persistence, if enabled (`--persist-dir`).
    disk: Option<StorageDisk>,

    /// Path to the file containing the active oracle session URL.
    oracle_url_file: String,

    /// Path to the directory containing static assets.
    static_dir: String,

    /// Account/invite/board-ownership database (Vision Round 5 F0).
    accounts: AccountsDb,

    /// Per-account connector control channels (Vision Round 5 F1a). Keyed by
    /// account id; the value holds every live connector that account has open.
    /// Used to push serve/unserve events and to report connector-online status.
    connectors: DashMap<String, ConnectorRegistry>,
}

impl ServerState {
    /// Create an empty server state using the given secret.
    pub async fn new(options: ServerOptions) -> Result<Self> {
        let secret = options.secret.unwrap_or_else(|| rand_alphanumeric(22));
        let mesh = match options.redis_url {
            Some(url) => Some(StorageMesh::new(&url, options.host.as_deref())?),
            None => None,
        };
        let disk = match options.persist_dir {
            Some(ref dir) => Some(StorageDisk::new(dir)?),
            None => None,
        };
        let oracle_url_file = options
            .oracle_url_file
            .unwrap_or_else(|| "/root/.sshx-oracle-url.txt".to_string());
        let static_dir = options
            .static_dir
            .unwrap_or_else(|| "build".to_string());
        let accounts = AccountsDb::new(options.persist_dir.as_deref()).await?;
        Ok(Self {
            mac: Hmac::new_from_slice(secret.as_bytes()).unwrap(),
            override_origin: options.override_origin,
            allowed_origin: options.allowed_origin.filter(|o| !o.is_empty()),
            insecure_cookies: options.insecure_cookies,
            store: DashMap::new(),
            mesh,
            disk,
            oracle_url_file,
            static_dir,
            accounts,
            connectors: DashMap::new(),
        })
    }

    /// Returns the account/invite/board-ownership database.
    pub fn accounts(&self) -> &AccountsDb {
        &self.accounts
    }

    /// Register a connector control channel for `account_id`. Returns the
    /// connection id (pass it back to [`Self::connector_unregister`] on
    /// disconnect) and the receiver the WS handler forwards to the socket.
    pub fn connector_register(
        &self,
        account_id: &str,
    ) -> (u64, mpsc::UnboundedReceiver<ConnectorEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut entry = self.connectors.entry(account_id.to_string()).or_default();
        let id = entry.next_id;
        entry.next_id += 1;
        entry.handles.push(ConnectorHandle { id, tx });
        (id, rx)
    }

    /// Drop a connector control channel by the id from
    /// [`Self::connector_register`]. Idempotent.
    pub fn connector_unregister(&self, account_id: &str, id: u64) {
        if let Some(mut entry) = self.connectors.get_mut(account_id) {
            entry.handles.retain(|h| h.id != id);
        }
    }

    /// How many connectors are live for `account_id` — the "connector online"
    /// signal the web surfaces.
    pub fn connector_count(&self, account_id: &str) -> usize {
        self.connectors.get(account_id).map_or(0, |e| e.handles.len())
    }

    /// Push an event to every live connector of `account_id`, dropping any whose
    /// receiver has closed (the connection died without unregistering). A no-op
    /// when the account has no connector online.
    pub fn connector_push(&self, account_id: &str, event: ConnectorEvent) {
        if let Some(mut entry) = self.connectors.get_mut(account_id) {
            entry.handles.retain(|h| h.tx.send(event.clone()).is_ok());
        }
    }

    /// Returns the message authentication code used for signing tokens.
    pub fn mac(&self) -> Hmac<Sha256> {
        self.mac.clone()
    }

    /// Returns the override origin for the Open() RPC.
    pub fn override_origin(&self) -> Option<String> {
        self.override_origin.clone()
    }

    /// Returns the allowed browser Origin for cross-origin-sensitive requests,
    /// if an allow-list is configured.
    pub fn allowed_origin(&self) -> Option<&str> {
        self.allowed_origin.as_deref()
    }

    /// Whether session cookies should be emitted without `Secure` (dev only).
    pub fn insecure_cookies(&self) -> bool {
        self.insecure_cookies
    }

    /// Returns the path to the oracle URL file.
    pub fn oracle_url_file(&self) -> &str {
        &self.oracle_url_file
    }

    /// Returns the path to the static directory.
    pub fn static_dir(&self) -> &str {
        &self.static_dir
    }

    /// Lookup a local session by name.
    pub fn lookup(&self, name: &str) -> Option<Arc<Session>> {
        self.store.get(name).map(|s| s.clone())
    }

    /// Returns the disk persistence layer, if enabled.
    pub fn disk(&self) -> Option<&StorageDisk> {
        self.disk.as_ref()
    }

    /// Names of all sessions currently live in memory.
    pub fn live_session_names(&self) -> Vec<String> {
        self.store.iter().map(|e| e.key().clone()).collect()
    }

    /// Insert a session into the local store.
    pub fn insert(&self, name: &str, session: Arc<Session>) {
        if let Some(disk) = &self.disk {
            let name = name.to_string();
            let session = session.clone();
            let disk = disk.clone();
            tokio::spawn(async move {
                disk.background_sync(&name, session).await;
            });
        }
        if let Some(mesh) = &self.mesh {
            let name = name.to_string();
            let session = session.clone();
            let mesh = mesh.clone();
            tokio::spawn(async move {
                mesh.background_sync(&name, session).await;
            });
        }
        if let Some(prev_session) = self.store.insert(name.to_string(), session) {
            prev_session.shutdown();
        }
    }

    /// Remove a session from the local store.
    pub fn remove(&self, name: &str) -> bool {
        if let Some((_, session)) = self.store.remove(name) {
            session.shutdown();
            true
        } else {
            false
        }
    }

    /// Close a session permanently on this and other servers.
    pub async fn close_session(&self, name: &str) -> Result<()> {
        self.remove(name);
        if let Some(mesh) = &self.mesh {
            mesh.mark_closed(name).await?;
        }
        if let Some(disk) = &self.disk {
            disk.delete(name); // permanent close = forget the board
        }
        Ok(())
    }

    /// Evict an idle session from memory WITHOUT deleting its persisted
    /// snapshot — the board "sleeps" on disk and can be reopened later.
    async fn evict_session(&self, name: &str) -> Result<()> {
        if let Some(disk) = &self.disk {
            if let Some(session) = self.lookup(name) {
                disk.save(name, &session.snapshot()?)?;
            }
            self.remove(name);
            return Ok(());
        }
        self.close_session(name).await
    }

    /// Restore a session from a snapshot and rehydrate per-shell backend
    /// ownership from the `backend_owners` table (owners aren't in the snapshot;
    /// a `Channel()` reconnect after restart wouldn't re-present the connector
    /// token, so without this per-shell enforcement would silently lapse across
    /// a restart).
    async fn restore_and_rehydrate(&self, name: &str, snapshot: &[u8]) -> Result<Arc<Session>> {
        let session = Arc::new(Session::restore(snapshot)?);
        match self.accounts.backend_owners(name).await {
            Ok(owners) => {
                for (backend_id, account_id) in owners {
                    session.set_backend_owner(BackendId(backend_id), account_id);
                }
            }
            Err(err) => error!(?err, board = %name, "failed to rehydrate backend owners"),
        }
        Ok(session)
    }

    /// Connect to a session by name from the `sshx` client, which provides the
    /// actual terminal backend.
    pub async fn backend_connect(&self, name: &str) -> Result<Option<Arc<Session>>> {
        if let Some(session) = self.lookup(name) {
            return Ok(Some(session));
        }

        if let Some(mesh) = &self.mesh {
            let (owner, snapshot) = mesh.get_owner_snapshot(name).await?;
            if let Some(snapshot) = snapshot {
                let session = self.restore_and_rehydrate(name, &snapshot).await?;
                self.insert(name, session.clone());
                if let Some(owner) = owner {
                    mesh.notify_transfer(name, &owner).await?;
                }
                return Ok(Some(session));
            }
        }

        if let Some(disk) = &self.disk {
            if let Some(snapshot) = disk.load(name) {
                let session = self.restore_and_rehydrate(name, &snapshot).await?;
                self.insert(name, session.clone());
                return Ok(Some(session));
            }
        }

        Ok(None)
    }

    /// Connect to a session from a web browser frontend, possibly redirecting.
    pub async fn frontend_connect(
        &self,
        name: &str,
    ) -> Result<Result<Arc<Session>, Option<String>>> {
        if let Some(session) = self.lookup(name) {
            return Ok(Ok(session));
        }

        // Wake a sleeping board from disk before consulting the mesh.
        if let Some(disk) = &self.disk {
            if let Some(snapshot) = disk.load(name) {
                let session = self.restore_and_rehydrate(name, &snapshot).await?;
                self.insert(name, session.clone());
                return Ok(Ok(session));
            }
        }

        if let Some(mesh) = &self.mesh {
            let mut owner = mesh.get_owner(name).await?;
            if owner.is_some() && owner.as_deref() == mesh.host() {
                // Do not redirect back to the same server.
                owner = None;
            }
            return Ok(Err(owner));
        }

        Ok(Err(None))
    }

    /// Listen for and remove sessions that are transferred away from this host.
    pub async fn listen_for_transfers(&self) {
        if let Some(mesh) = &self.mesh {
            let mut transfers = pin!(mesh.listen_for_transfers());
            while let Some(name) = transfers.next().await {
                self.remove(&name);
            }
        }
    }

    /// Close all sessions that have been disconnected for too long.
    pub async fn close_old_sessions(&self) {
        loop {
            time::sleep(DISCONNECTED_SESSION_EXPIRY / 5).await;
            let mut to_close = Vec::new();
            for entry in &self.store {
                let session = entry.value();
                if session.last_accessed().elapsed() > DISCONNECTED_SESSION_EXPIRY {
                    to_close.push(entry.key().clone());
                }
            }
            for name in to_close {
                if let Err(err) = self.evict_session(&name).await {
                    error!(?err, "failed to evict old session {name}");
                }
            }
        }
    }

    /// Send a graceful shutdown signal to every session.
    pub fn shutdown(&self) {
        for entry in &self.store {
            entry.value().shutdown();
        }
    }
}
