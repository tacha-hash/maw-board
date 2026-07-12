//! Account/invite/board-ownership storage (Vision Round 5 F0).
//!
//! docs/vision-round5-f0-design.md §2 (le-workboard repo) is the design
//! contract this schema implements — see that doc before changing table
//! shape, since Le's CLI invite tool and migration script depend on it too.
//!
//! This module owns the connection pool and migrations only. Query/business
//! logic (login, invite redemption, membership checks) lands in the auth
//! middleware work that follows this commit — kept out of here so the
//! schema can be reviewed as its own small, contract-boundary change.

use anyhow::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

/// Account/board-ownership database, backed by SQLite.
#[derive(Clone)]
pub struct AccountsDb {
    pool: SqlitePool,
}

impl AccountsDb {
    /// Open (creating if needed) `accounts.db` next to the given persist
    /// directory, or an in-memory database if none is configured — so tests
    /// and `--persist-dir`-less dev runs don't need a real file on disk.
    pub async fn new(persist_dir: Option<&str>) -> Result<Self> {
        let connect_options = match persist_dir {
            Some(dir) => {
                std::fs::create_dir_all(dir)?;
                let path = std::path::Path::new(dir).join("accounts.db");
                SqliteConnectOptions::new().filename(path).create_if_missing(true)
            }
            None => "sqlite::memory:".parse::<SqliteConnectOptions>()?,
        };

        let pool = SqlitePoolOptions::new()
            // SQLite only allows one writer at a time; a single connection
            // avoids SQLITE_BUSY errors under the light write load an
            // accounts DB for a friend-group actually sees, without adding
            // WAL-mode config for a size of data that doesn't need it.
            .max_connections(1)
            .connect_with(connect_options)
            .await?;

        sqlx::migrate!("./migrations").run(&pool).await?;

        Ok(Self { pool })
    }

    /// Returns the underlying connection pool, for the auth/ACL code that
    /// follows this commit.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
