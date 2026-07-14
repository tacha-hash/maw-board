-- Vision Round 5 F0.5 — per-board member capabilities (docs/vision-round5-f05-
-- account-permissions-design.md). Two boolean flags on each membership row:
--
--   can_edit  — may modify board items (BoardPut/Move/Delete). Enforced at the
--               WS handshake as `can_write` (the existing check_write_permission
--               choke-point), so a view-only member's edits are rejected server-
--               side. Backfill DEFAULT 1 preserves F0 behavior (every member
--               could write) — no existing member loses access.
--   can_order — may dispatch Work Orders to agents. DEFAULT 0: the strongest
--               grant (it makes agents act) must be given deliberately. Gated in
--               the frontend for F0.5; server/connector enforcement is F1.
--
-- The board OWNER (board_members.role = 'owner') implicitly has every capability
-- and is never gated by these flags — callers check role first. Neither flag can
-- ever grant type-into-terminal, which stays owner-only via may_mutate_shell
-- (the invariant; capabilities are a separate, additive layer).

ALTER TABLE board_members ADD COLUMN can_edit  INTEGER NOT NULL DEFAULT 1;
ALTER TABLE board_members ADD COLUMN can_order INTEGER NOT NULL DEFAULT 0;
