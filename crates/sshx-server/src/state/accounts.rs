//! Account/invite/board-ownership storage (Vision Round 5 F0).
//!
//! docs/vision-round5-f0-design.md §2 (le-workboard repo) is the design
//! contract this schema implements — see that doc before changing table
//! shape, since Le's CLI invite tool and migration script depend on it too.
//!
//! This module owns the connection pool, migrations, and the query layer for
//! accounts / invites / board membership — the single place any code (routes,
//! auth gate, and the `invite`/`migrate-vr5` CLI subcommands) reaches the
//! accounts DB. Keeping every writer on this one layer, rather than a separate
//! tool that re-encodes the schema, is deliberate: schema duplicated across a
//! second implementation is exactly the contract-drift that bit the Work Order
//! feature (Le, 2026-07-12).

use anyhow::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};

/// One account row (only the columns callers actually need).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Account {
    /// Opaque alphanumeric id (not enumerable).
    pub id: String,
    /// Unique login name.
    pub username: String,
    /// Argon2id PHC string, or `None` for a passkey-only account (F0.5).
    pub password_hash: Option<String>,
}

/// Outcome of redeeming an invite to create an account.
#[derive(Debug)]
pub enum JoinOutcome {
    /// Account created and invite consumed.
    Created(Account),
    /// The invite code doesn't exist or was already used.
    InvalidInvite,
    /// The requested username is already taken.
    UsernameTaken,
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

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
                SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true)
                    // The server holds this DB open while the `invite`/
                    // `migrate-vr5` subcommands open it from a second process;
                    // wait for the writer lock instead of erroring SQLITE_BUSY.
                    .busy_timeout(std::time::Duration::from_secs(5))
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

    /// Returns the underlying connection pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // ── Accounts ────────────────────────────────────────────────────────────

    /// Look up an account by its unique username.
    pub async fn account_by_username(&self, username: &str) -> Result<Option<Account>> {
        let account = sqlx::query_as::<_, Account>(
            "SELECT id, username, password_hash FROM accounts WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(account)
    }

    /// True if an account with this id currently exists. Cheap identity
    /// existence check for the auth gate (the session cookie proves *who*, this
    /// confirms the account wasn't since deleted).
    pub async fn account_exists(&self, id: &str) -> Result<bool> {
        let found: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM accounts WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(found.is_some())
    }

    /// The account id owning a connector bearer token, looked up by the token's
    /// at-rest hash (see [`crate::auth::connector_token_hash`]). `None` if no
    /// account holds that token.
    pub async fn account_id_by_connector_token(&self, token_hash: &str) -> Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT id FROM accounts WHERE connector_token = ?")
                .bind(token_hash)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|r| r.0))
    }

    /// Set (or rotate) the connector token hash for an account, by username.
    /// Returns `false` if no such username exists.
    pub async fn set_connector_token(&self, username: &str, token_hash: &str) -> Result<bool> {
        let res = sqlx::query("UPDATE accounts SET connector_token = ? WHERE username = ?")
            .bind(token_hash)
            .bind(username)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Directly create an account, bypassing the invite flow — for the
    /// `migrate-vr5` bootstrap of the "louis" account only (the chicken-egg:
    /// the very first account can't be created *through* an invite, since
    /// invites.created_by references an account that doesn't exist yet). Every
    /// other account is born via [`redeem_invite`](Self::redeem_invite).
    /// Idempotent: a no-op if the username already exists.
    pub async fn bootstrap_account(&self, username: &str, password_hash: &str) -> Result<Account> {
        if let Some(existing) = self.account_by_username(username).await? {
            return Ok(existing);
        }
        let id = sshx_core::rand_alphanumeric(16);
        sqlx::query(
            "INSERT INTO accounts (id, username, password_hash, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(username)
        .bind(password_hash)
        .bind(now_unix_secs())
        .execute(&self.pool)
        .await?;
        Ok(Account {
            id,
            username: username.to_string(),
            password_hash: Some(password_hash.to_string()),
        })
    }

    // ── Invites ─────────────────────────────────────────────────────────────

    /// Create a new single-use invite attributed to `created_by`, returning the
    /// generated code.
    pub async fn create_invite(&self, created_by: &str) -> Result<String> {
        let code = sshx_core::rand_alphanumeric(10);
        sqlx::query("INSERT INTO invites (code, created_by, created_at) VALUES (?, ?, ?)")
            .bind(&code)
            .bind(created_by)
            .bind(now_unix_secs())
            .execute(&self.pool)
            .await?;
        Ok(code)
    }

    /// Redeem an invite to create an account, atomically: the invite must exist
    /// and be unused, and the username must be free. Consuming the invite and
    /// creating the account happen in one transaction so a code can never mint
    /// two accounts even under a race (also backstopped by the UNIQUE
    /// constraints).
    pub async fn redeem_invite(
        &self,
        code: &str,
        username: &str,
        password_hash: &str,
    ) -> Result<JoinOutcome> {
        let mut tx = self.pool.begin().await?;

        // Invite must exist and be unclaimed.
        let invite: Option<(String,)> =
            sqlx::query_as("SELECT code FROM invites WHERE code = ? AND used_by IS NULL")
                .bind(code)
                .fetch_optional(&mut *tx)
                .await?;
        if invite.is_none() {
            return Ok(JoinOutcome::InvalidInvite);
        }

        // Username must be free.
        let taken: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM accounts WHERE username = ?")
            .bind(username)
            .fetch_optional(&mut *tx)
            .await?;
        if taken.is_some() {
            return Ok(JoinOutcome::UsernameTaken);
        }

        let id = sshx_core::rand_alphanumeric(16);
        let now = now_unix_secs();
        sqlx::query(
            "INSERT INTO accounts (id, username, password_hash, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(username)
        .bind(password_hash)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE invites SET used_by = ?, used_at = ? WHERE code = ?")
            .bind(&id)
            .bind(now)
            .bind(code)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(JoinOutcome::Created(Account {
            id,
            username: username.to_string(),
            password_hash: Some(password_hash.to_string()),
        }))
    }

    // ── Boards & membership ─────────────────────────────────────────────────

    /// Record ownership of a board and add the owner as a member, atomically.
    /// Idempotent via `INSERT OR IGNORE` so re-running the migration or
    /// re-creating an already-tracked board is harmless.
    pub async fn create_board(&self, name: &str, owner_account_id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let now = now_unix_secs();
        sqlx::query(
            "INSERT OR IGNORE INTO boards (name, owner_account_id, created_at) VALUES (?, ?, ?)",
        )
        .bind(name)
        .bind(owner_account_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO board_members (board_name, account_id, role, added_at) \
             VALUES (?, ?, 'owner', ?)",
        )
        .bind(name)
        .bind(owner_account_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Forget a board entirely: drop its membership rows, ownership row, and
    /// backend-owner rows. Called when a board is permanently deleted.
    pub async fn forget_board(&self, name: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM board_members WHERE board_name = ?")
            .bind(name)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM backend_owners WHERE board_name = ?")
            .bind(name)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM boards WHERE name = ?")
            .bind(name)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Persist a backend's owning account so per-shell ownership survives a
    /// server restart (a `Channel()` reconnect doesn't re-present the connector
    /// token). Keyed on (board, backend_id); re-registering the same id
    /// replaces the row.
    pub async fn set_backend_owner(
        &self,
        board_name: &str,
        backend_id: u32,
        account_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO backend_owners (board_name, backend_id, account_id) \
             VALUES (?, ?, ?)",
        )
        .bind(board_name)
        .bind(backend_id as i64)
        .bind(account_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The (backend_id, account_id) owners recorded for a board, used to
    /// rehydrate backend ownership after a snapshot restore.
    pub async fn backend_owners(&self, board_name: &str) -> Result<Vec<(u32, String)>> {
        let rows: Vec<(i64, String)> =
            sqlx::query_as("SELECT backend_id, account_id FROM backend_owners WHERE board_name = ?")
                .bind(board_name)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(id, acc)| (id as u32, acc)).collect())
    }

    /// The owning account id of a board, if the board is tracked.
    pub async fn board_owner(&self, name: &str) -> Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT owner_account_id FROM boards WHERE name = ?")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|r| r.0))
    }

    /// Whether an account may access a board — a live membership check, never
    /// cached in the session cookie, so removing the row revokes access
    /// immediately even while the cookie is still otherwise valid (Le MUST-2).
    pub async fn is_member(&self, board_name: &str, account_id: &str) -> Result<bool> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM board_members WHERE board_name = ? AND account_id = ?",
        )
        .bind(board_name)
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Names of every board an account is a member of (for the per-account
    /// lobby listing).
    pub async fn boards_for_account(&self, account_id: &str) -> Result<Vec<String>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT board_name FROM board_members WHERE account_id = ?")
                .bind(account_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Add a member to a board by username (the F0 "share to username" flow).
    /// Returns `false` if no such username exists; idempotent on re-add.
    pub async fn add_member_by_username(&self, board_name: &str, username: &str) -> Result<bool> {
        let Some(account) = self.account_by_username(username).await? else {
            return Ok(false);
        };
        sqlx::query(
            "INSERT OR IGNORE INTO board_members (board_name, account_id, role, added_at) \
             VALUES (?, ?, 'member', ?)",
        )
        .bind(board_name)
        .bind(&account.id)
        .bind(now_unix_secs())
        .execute(&self.pool)
        .await?;
        Ok(true)
    }

    /// Remove a member from a board by username. Returns `false` if the
    /// username doesn't exist. The owner cannot be removed this way (their
    /// 'owner' row is left intact).
    pub async fn remove_member_by_username(
        &self,
        board_name: &str,
        username: &str,
    ) -> Result<bool> {
        let Some(account) = self.account_by_username(username).await? else {
            return Ok(false);
        };
        sqlx::query(
            "DELETE FROM board_members \
             WHERE board_name = ? AND account_id = ? AND role != 'owner'",
        )
        .bind(board_name)
        .bind(&account.id)
        .execute(&self.pool)
        .await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> AccountsDb {
        AccountsDb::new(None).await.unwrap()
    }

    #[tokio::test]
    async fn invite_redeem_is_single_use_and_username_unique() {
        let db = db().await;
        let louis = db.bootstrap_account("louis", "hash-l").await.unwrap();
        let code = db.create_invite(&louis.id).await.unwrap();

        // Happy path: fresh code + free username → account created.
        let outcome = db.redeem_invite(&code, "alice", "hash-a").await.unwrap();
        let alice = match outcome {
            JoinOutcome::Created(a) => a,
            other => panic!("expected Created, got {other:?}"),
        };
        assert_eq!(alice.username, "alice");
        assert!(db.account_exists(&alice.id).await.unwrap());

        // Same code again → already used, no second account.
        let reuse = db.redeem_invite(&code, "bob", "hash-b").await.unwrap();
        assert!(matches!(reuse, JoinOutcome::InvalidInvite));

        // Fresh code but a taken username → rejected, invite left unused.
        let code2 = db.create_invite(&louis.id).await.unwrap();
        let dup = db.redeem_invite(&code2, "alice", "hash-x").await.unwrap();
        assert!(matches!(dup, JoinOutcome::UsernameTaken));
        // code2 must still be redeemable since the failed attempt didn't consume it.
        let ok = db.redeem_invite(&code2, "carol", "hash-c").await.unwrap();
        assert!(matches!(ok, JoinOutcome::Created(_)));
    }

    #[tokio::test]
    async fn nonexistent_invite_is_rejected() {
        let db = db().await;
        let outcome = db.redeem_invite("nope", "eve", "hash-e").await.unwrap();
        assert!(matches!(outcome, JoinOutcome::InvalidInvite));
    }

    #[tokio::test]
    async fn bootstrap_is_idempotent() {
        let db = db().await;
        let a = db.bootstrap_account("louis", "hash-1").await.unwrap();
        let b = db.bootstrap_account("louis", "hash-2").await.unwrap();
        assert_eq!(a.id, b.id); // same account, not a duplicate
    }

    #[tokio::test]
    async fn board_membership_and_revocation() {
        let db = db().await;
        let louis = db.bootstrap_account("louis", "h").await.unwrap();
        let code = db.create_invite(&louis.id).await.unwrap();
        let JoinOutcome::Created(friend) = db.redeem_invite(&code, "friend", "h").await.unwrap()
        else {
            panic!("join failed");
        };

        db.create_board("boardA", &louis.id).await.unwrap();
        // Owner is a member and sees it; the friend is not, yet.
        assert!(db.is_member("boardA", &louis.id).await.unwrap());
        assert!(!db.is_member("boardA", &friend.id).await.unwrap());
        assert_eq!(db.board_owner("boardA").await.unwrap().as_deref(), Some(louis.id.as_str()));

        // Share to the friend by username.
        assert!(db.add_member_by_username("boardA", "friend").await.unwrap());
        assert!(db.is_member("boardA", &friend.id).await.unwrap());
        assert!(db.boards_for_account(&friend.id).await.unwrap().contains(&"boardA".to_string()));
        // Sharing to a nonexistent username reports false, adds nothing.
        assert!(!db.add_member_by_username("boardA", "ghost").await.unwrap());

        // Revoke: friend loses access immediately.
        assert!(db.remove_member_by_username("boardA", "friend").await.unwrap());
        assert!(!db.is_member("boardA", &friend.id).await.unwrap());
        // The owner can't be removed via the member API (role='owner' protected).
        db.remove_member_by_username("boardA", "louis").await.unwrap();
        assert!(db.is_member("boardA", &louis.id).await.unwrap());
    }
}
