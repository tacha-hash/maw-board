//! Durable on-disk persistence for sessions ("board = project").
//!
//! Mirrors the `StorageMesh` background-sync pattern, but writes snapshots to
//! a local directory with no TTL, so boards survive server restarts. Enabled
//! with the `--persist-dir` flag; independent of (and composable with) Redis.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::Result;
use tokio::time;
use tracing::error;

use crate::session::Session;

/// Interval for syncing session state to disk (same cadence as mesh).
const DISK_SYNC_INTERVAL: Duration = Duration::from_secs(20);

/// Durable session storage in a local directory.
#[derive(Clone)]
pub struct StorageDisk {
    dir: PathBuf,
}

/// Metadata about one persisted board, for the lobby listing.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PersistedBoard {
    /// Session name (file stem).
    pub name: String,
    /// Last modification time, in seconds since the Unix epoch.
    pub modified: u64,
    /// Snapshot size in bytes.
    pub size: u64,
}

fn safe_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl StorageDisk {
    /// Create the storage directory if needed and return a handle.
    pub fn new(dir: &str) -> Result<Self> {
        let dir = PathBuf::from(dir);
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path_for(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.bin"))
    }

    /// Atomically write a snapshot for the session.
    pub fn save(&self, name: &str, snapshot: &[u8]) -> Result<()> {
        if !safe_name(name) {
            anyhow::bail!("unsafe session name for disk persistence: {name:?}");
        }
        let tmp = self.dir.join(format!("{name}.bin.tmp"));
        fs::write(&tmp, snapshot)?;
        fs::rename(&tmp, self.path_for(name))?;
        Ok(())
    }

    /// Load a persisted snapshot, if one exists.
    pub fn load(&self, name: &str) -> Option<Vec<u8>> {
        if !safe_name(name) {
            return None;
        }
        fs::read(self.path_for(name)).ok()
    }

    /// Remove a persisted snapshot (session closed permanently).
    /// Also removes the escrowed key file, if any — key escrow follows the
    /// board's lifecycle.
    pub fn delete(&self, name: &str) {
        if safe_name(name) {
            let _ = fs::remove_file(self.path_for(name));
            let _ = fs::remove_file(self.dir.join(format!("{name}.key")));
        }
    }

    /// List all persisted boards, newest first.
    pub fn list(&self) -> Vec<PersistedBoard> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut boards: Vec<PersistedBoard> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let path = e.path();
                let name = path.file_stem()?.to_str()?.to_string();
                if path.extension()?.to_str()? != "bin" || !safe_name(&name) {
                    return None;
                }
                let meta = e.metadata().ok()?;
                let modified = meta
                    .modified()
                    .ok()?
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .ok()?
                    .as_secs();
                Some(PersistedBoard { name, modified, size: meta.len() })
            })
            .collect();
        boards.sort_by(|a, b| b.modified.cmp(&a.modified));
        boards
    }

    /// Periodically write the session snapshot to disk until it terminates.
    /// Mirrors `StorageMesh::background_sync`.
    pub async fn background_sync(&self, name: &str, session: Arc<Session>) {
        let mut interval = time::interval(DISK_SYNC_INTERVAL);
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = session.sync_now_wait() => {}
                _ = session.terminated() => break,
            }
            match session.snapshot() {
                Ok(snapshot) => {
                    if let Err(err) = self.save(name, &snapshot) {
                        error!(?err, "failed to persist session {name} to disk");
                    }
                }
                Err(err) => error!(?err, "failed to snapshot session {name}"),
            }
        }
        // Final write on graceful termination is intentionally skipped: session
        // termination means "closed", and close_session() deletes the file.
    }

    /// The directory this storage writes into (for logging/tests).
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}
