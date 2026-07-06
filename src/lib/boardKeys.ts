/**
 * @file Local memory of board encryption keys, keyed by session name.
 *
 * The server can never hand back a board's key — sshx's whole design is E2E,
 * keys live only in the URL fragment (docs/lobby-ui-design.md, le-workboard).
 * This is the only place that gap can be closed: whichever page actually
 * has the key at some point (the lobby after creating a board, or a session
 * page opened via a full URL/pasted key) writes it here; the lobby's "Open"
 * button reads it back.
 *
 * Bug found live (Louis, 2026-07-06 ค่ำ): only the lobby's New Board flow
 * wrote here — opening a board via a full URL never did, so the lobby kept
 * showing "Need key" for boards you very much had the key for. Both write
 * sites now share this one module so they can't diverge like that again.
 *
 * Note: localStorage is origin-scoped — this map is only consistent if you
 * always reach the board through the same origin (e.g. always the
 * tailscale hostname, never mixed with localhost/127.0.0.1).
 */

const KEY_STORAGE = "oracle-board-keys";

export function loadKeyMap(): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(KEY_STORAGE) ?? "{}");
  } catch {
    return {};
  }
}

export function keyFor(name: string): string | undefined {
  return loadKeyMap()[name];
}

export function saveKey(name: string, key: string) {
  const map = loadKeyMap();
  map[name] = key;
  localStorage.setItem(KEY_STORAGE, JSON.stringify(map));
}

export function removeKey(name: string) {
  const map = loadKeyMap();
  delete map[name];
  localStorage.setItem(KEY_STORAGE, JSON.stringify(map));
}
