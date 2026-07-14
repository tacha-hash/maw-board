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

/// A member's capabilities on one board (F0.5). The board OWNER (role='owner')
/// implicitly has every capability regardless of the flags — always resolve
/// through [`may_edit`](Self::may_edit)/[`may_order`](Self::may_order), never the
/// raw booleans.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MemberCapability {
    /// 'owner' | 'member'.
    pub role: String,
    /// May modify board items (enforced as `can_write` at the WS handshake).
    pub can_edit: bool,
    /// May dispatch Work Orders to agents.
    pub can_order: bool,
}

impl MemberCapability {
    /// May this member modify board items? Owner always; otherwise per can_edit.
    pub fn may_edit(&self) -> bool {
        self.role == "owner" || self.can_edit
    }
    /// May this member dispatch Work Orders? Owner always; otherwise per can_order.
    pub fn may_order(&self) -> bool {
        self.role == "owner" || self.can_order
    }
}

/// One board member row for the owner's member-management panel.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct MemberRow {
    /// The member's login name.
    pub username: String,
    /// 'owner' | 'member'.
    pub role: String,
    /// May modify board items.
    pub can_edit: bool,
    /// May dispatch Work Orders.
    pub can_order: bool,
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

    /// Look up an account by its opaque id (the id carried in a session cookie).
    pub async fn account_by_id(&self, id: &str) -> Result<Option<Account>> {
        let account = sqlx::query_as::<_, Account>(
            "SELECT id, username, password_hash FROM accounts WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(account)
    }

    /// The account's session epoch (unix secs), or `None` if the account no
    /// longer exists. A session cookie is valid only while its signed
    /// `issued_at >= session_epoch`; the epoch is bumped on password change /
    /// "logout everywhere", so this one read does double duty for the auth gate
    /// (account-exists AND not-a-stale-session) — see [`crate::auth`] and the
    /// `session_epoch` column comment in migrations/0001_accounts.sql.
    pub async fn account_session_epoch(&self, id: &str) -> Result<Option<i64>> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT session_epoch FROM accounts WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|r| r.0))
    }

    /// Set a new password hash AND bump `session_epoch` to now, atomically —
    /// changing a password invalidates every session issued before this moment
    /// (logout everywhere). The caller re-mints its own cookie afterwards so the
    /// account that changed the password stays logged in. Returns `false` if no
    /// such account id exists.
    pub async fn set_password_and_bump_epoch(&self, id: &str, new_hash: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE accounts SET password_hash = ?, session_epoch = ? WHERE id = ?",
        )
        .bind(new_hash)
        .bind(now_unix_secs())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
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

    /// Set (or rotate) the connector token hash for an account, by id (the id a
    /// session cookie carries — used by the `/account` rotate endpoint). Returns
    /// `false` if no such account id exists.
    pub async fn set_connector_token_by_id(&self, id: &str, token_hash: &str) -> Result<bool> {
        let res = sqlx::query("UPDATE accounts SET connector_token = ? WHERE id = ?")
            .bind(token_hash)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Whether an account currently has a connector token configured. The raw
    /// token is unrecoverable (only its hash is stored), so the `/account` UI can
    /// show "configured / not configured" but never re-display the token.
    pub async fn connector_token_configured(&self, id: &str) -> Result<bool> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM accounts WHERE id = ? AND connector_token IS NOT NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
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

    /// A member's capabilities on a board, or `None` if not a member. Used by
    /// the WS handshake (can_write = cap.may_edit()) and the order gate.
    pub async fn member_capabilities(
        &self,
        board_name: &str,
        account_id: &str,
    ) -> Result<Option<MemberCapability>> {
        let row = sqlx::query_as::<_, MemberCapability>(
            "SELECT role, can_edit, can_order FROM board_members \
             WHERE board_name = ? AND account_id = ?",
        )
        .bind(board_name)
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Every member of a board with role + capabilities (for the owner's member
    /// panel), joined to `accounts` for the username. Owner listed first.
    pub async fn list_members(&self, board_name: &str) -> Result<Vec<MemberRow>> {
        let rows = sqlx::query_as::<_, MemberRow>(
            "SELECT a.username, m.role, m.can_edit, m.can_order \
             FROM board_members m JOIN accounts a ON a.id = m.account_id \
             WHERE m.board_name = ? ORDER BY (m.role = 'owner') DESC, a.username",
        )
        .bind(board_name)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Set a member's capabilities by username. The owner's row is protected
    /// (`role != 'owner'`), so an owner can never accidentally strip their own
    /// access. Returns `false` if the username doesn't exist.
    pub async fn set_member_capabilities(
        &self,
        board_name: &str,
        username: &str,
        can_edit: bool,
        can_order: bool,
    ) -> Result<bool> {
        let Some(account) = self.account_by_username(username).await? else {
            return Ok(false);
        };
        sqlx::query(
            "UPDATE board_members SET can_edit = ?, can_order = ? \
             WHERE board_name = ? AND account_id = ? AND role != 'owner'",
        )
        .bind(can_edit)
        .bind(can_order)
        .bind(board_name)
        .bind(&account.id)
        .execute(&self.pool)
        .await?;
        Ok(true)
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
    async fn password_change_updates_hash_and_bumps_epoch() {
        let db = db().await;
        let acct = db.bootstrap_account("dave", "old-hash").await.unwrap();
        // Fresh account: epoch starts at the schema default 0.
        assert_eq!(db.account_session_epoch(&acct.id).await.unwrap(), Some(0));
        assert_eq!(
            db.account_by_id(&acct.id).await.unwrap().unwrap().password_hash.as_deref(),
            Some("old-hash")
        );

        // Change → hash replaced AND epoch bumped to a real unix timestamp.
        assert!(db.set_password_and_bump_epoch(&acct.id, "new-hash").await.unwrap());
        assert_eq!(
            db.account_by_id(&acct.id).await.unwrap().unwrap().password_hash.as_deref(),
            Some("new-hash")
        );
        let epoch = db.account_session_epoch(&acct.id).await.unwrap().unwrap();
        assert!(epoch > 0, "epoch should be bumped to a unix-secs timestamp, got {epoch}");

        // Unknown id: no row → None / no-op.
        assert_eq!(db.account_session_epoch("nope").await.unwrap(), None);
        assert!(!db.set_password_and_bump_epoch("nope", "x").await.unwrap());
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
    async fn member_capabilities_default_owner_and_update() {
        let db = db().await;
        let louis = db.bootstrap_account("louis", "h").await.unwrap();
        let code = db.create_invite(&louis.id).await.unwrap();
        let JoinOutcome::Created(friend) = db.redeem_invite(&code, "friend", "h").await.unwrap()
        else {
            panic!("join failed");
        };
        db.create_board("b", &louis.id).await.unwrap();
        db.add_member_by_username("b", "friend").await.unwrap();

        // Owner: implicitly all capabilities regardless of the raw flags.
        let owner = db.member_capabilities("b", &louis.id).await.unwrap().unwrap();
        assert_eq!(owner.role, "owner");
        assert!(owner.may_edit() && owner.may_order());

        // Fresh member: backfill default can_edit=1, can_order=0.
        let m = db.member_capabilities("b", &friend.id).await.unwrap().unwrap();
        assert_eq!(m.role, "member");
        assert!(m.can_edit && !m.can_order);
        assert!(m.may_edit() && !m.may_order());

        // Downgrade to view-only, then grant order.
        assert!(db.set_member_capabilities("b", "friend", false, false).await.unwrap());
        let m = db.member_capabilities("b", &friend.id).await.unwrap().unwrap();
        assert!(!m.may_edit() && !m.may_order());
        assert!(db.set_member_capabilities("b", "friend", true, true).await.unwrap());
        let m = db.member_capabilities("b", &friend.id).await.unwrap().unwrap();
        assert!(m.may_edit() && m.may_order());

        // The owner row is protected from capability edits.
        db.set_member_capabilities("b", "louis", false, false).await.unwrap();
        let owner = db.member_capabilities("b", &louis.id).await.unwrap().unwrap();
        assert!(owner.may_edit() && owner.may_order(), "owner caps unaffected");

        // list_members: owner first, both present.
        let members = db.list_members("b").await.unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].role, "owner");
        assert_eq!(members[0].username, "louis");

        // Unknown username → false; a non-member → None.
        assert!(!db.set_member_capabilities("b", "ghost", true, true).await.unwrap());
        let outsider = db.bootstrap_account("outsider", "h").await.unwrap();
        assert!(db.member_capabilities("b", &outsider.id).await.unwrap().is_none());
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
