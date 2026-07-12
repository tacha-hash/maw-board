-- Vision Round 5 F0 — accounts, invites, board ownership/membership.
-- docs/vision-round5-f0-design.md §2 (le-workboard repo) is the design contract.
-- id columns use sshx_core::rand_alphanumeric, not autoincrement, so ids
-- can't be enumerated/guessed the way a sequential integer could.

CREATE TABLE accounts (
    id            TEXT PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT,                  -- argon2id encoded string; NULL if passkey-only later
    passkey_credential TEXT,             -- nullable, unused until F0.5 (WebAuthn)
    -- Bearer token for the oracle connector (board-bridge.ts) and other
    -- programmatic clients — separate from the browser's session cookie.
    -- Nullable/unset until an account requests one (F0 task 3, key-via-API).
    connector_token TEXT UNIQUE,
    -- Session epoch: bumped on "logout everywhere" / password change. A cookie
    -- is valid only while its signed issued_at >= this. Not checked in F0 —
    -- prepared now (same pattern as connector_token) so the future check is a
    -- free comparison riding the per-request board-access DB read, with no
    -- second migration (Le review future-proof, 2026-07-12).
    session_epoch INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL
);

CREATE TABLE invites (
    code          TEXT PRIMARY KEY,
    created_by    TEXT NOT NULL REFERENCES accounts(id),
    used_by       TEXT REFERENCES accounts(id),
    created_at    INTEGER NOT NULL,
    used_at       INTEGER
);

CREATE TABLE boards (
    name              TEXT PRIMARY KEY,   -- matches the session name already used in state.rs
    owner_account_id  TEXT NOT NULL REFERENCES accounts(id),
    created_at        INTEGER NOT NULL
);

CREATE TABLE board_members (
    board_name   TEXT NOT NULL REFERENCES boards(name),
    account_id   TEXT NOT NULL REFERENCES accounts(id),
    role         TEXT NOT NULL,  -- 'owner' | 'member'
    added_at     INTEGER NOT NULL,
    PRIMARY KEY (board_name, account_id)
);

-- F1 prep — not queried by F0 code. Backends are runtime state (the bridge
-- spawns them per run; they aren't persisted on disk), so nothing is
-- backfilled here at migration time — this table is populated at runtime via
-- register_backend() once per-shell ownership lands (F1). Declared now so F1
-- needs no second schema migration just to add it. (Le review, 2026-07-12.)
CREATE TABLE backend_owners (
    board_name    TEXT NOT NULL,
    backend_id    INTEGER NOT NULL,   -- matches sshx_core::BackendId
    account_id    TEXT NOT NULL REFERENCES accounts(id),
    PRIMARY KEY (board_name, backend_id)
);
