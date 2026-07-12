//! Account authentication primitives (Vision Round 5 F0).
//!
//! Two clearly separated cryptographic concerns live near each other in this
//! server, and Le's design review (docs/vision-round5-f0-design.md, open
//! question 3) called out the risk of confusing them:
//!
//! - **E2E key derivation** ([`crate::web`]'s `encrypted_zeros_for`) — Argon2id
//!   with a *fixed, shared* salt (`ENCRYPT_SALT`). It MUST be deterministic
//!   across every client so the same board key derives the same AES key
//!   everywhere. That is terminal-content encryption, unrelated to accounts.
//!
//! - **Account password hashing** ([`hash_account_password`] here) — Argon2id
//!   with a *random, per-account* salt embedded in a PHC string. It MUST NOT
//!   share the fixed salt above, or every account's hash would be trivially
//!   rainbow-tableable.
//!
//! Keeping them in different modules with unmistakable names is deliberate:
//! nobody should be able to reach for "the argon2 function" and get the wrong
//! salt behavior.
//!
//! Browser sessions use a stateless HMAC-signed cookie (signed with the
//! server's `SSHX_SECRET` MAC), rather than a server-side session store —
//! fewer moving parts on the auth-critical path at F0's scale. See
//! [`mint_session_cookie`].

use anyhow::{anyhow, Result};
use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac::{Hmac, Mac as _};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Name of the browser session cookie.
pub const SESSION_COOKIE: &str = "sshx_session";

/// Domain-separation tag prefixed to every session-cookie signature. Signed
/// with the SAME server MAC (`SSHX_SECRET`) as `grpc::join_token`, so the tag
/// MUST be distinct from that chain (`<name>` then `b"join"`) or a join token
/// could be replayed as a session cookie and vice versa (Le review MUST-1,
/// 2026-07-12). This tag is a fixed prefix and join_token's input never begins
/// with it, so the two MAC inputs can never collide.
const SESSION_SIG_TAG: &[u8] = b"session:v1\0";

/// Identity proven by a valid session cookie. Deliberately holds ONLY identity
/// — never board membership. "Can this account touch this board?" is a live
/// `board_members` query per request (Le review MUST-2), so revoking access is
/// as simple as deleting a membership row; the cookie is never consulted for
/// authorization, only authentication.
#[derive(Debug, Clone)]
pub struct SessionClaims {
    /// The authenticated account id.
    pub account_id: String,
    /// When this cookie was issued (unix secs). Carried inside the signature
    /// so it can't be forged; unused for now, but a future
    /// "logout-everywhere / invalidate-on-password-change" check compares it
    /// against `accounts.session_epoch` for free on the board-access DB read
    /// that already happens per request (Le review future-proof).
    pub issued_at: u64,
}

/// Hash an account password with Argon2id and a fresh random salt, returning a
/// self-describing PHC string (`$argon2id$...`) safe to store verbatim.
///
/// This is NOT the E2E board-key derivation — see the module docs. Do not
/// route board keys through here or account passwords through that path.
pub fn hash_account_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow!("argon2 password hashing failed: {e}"))
}

/// Verify a password against a stored PHC hash in constant time (the argon2
/// verifier compares the derived hashes, not the raw strings). Returns false
/// for any malformed stored hash rather than erroring.
pub fn verify_account_password(password: &str, stored_phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored_phc) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// The at-rest representation of a connector bearer token: base64url of its
/// SHA-256. Stored in `accounts.connector_token`; the server hashes a presented
/// `Authorization: Bearer` token and looks it up. Unsalted SHA-256 is
/// sufficient here (and lets us look up by value) precisely because a connector
/// token is high-entropy random — unlike a low-entropy password, it isn't
/// rainbow-tableable, so it needs no per-value salt or slow KDF.
pub fn connector_token_hash(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
}

/// Mint a signed, stateless session cookie binding `account_id` for
/// `ttl_secs`. Value format: `v1:<account_id>:<issued_at>:<expires>:<b64url
/// (hmac)>`, signed with the server MAC. The `:` delimiters give unambiguous
/// field separation (chained MAC updates of adjacent variable-length fields
/// would not — `"a" ++ "12"` and `"a1" ++ "2"` collide); `account_id` is
/// alphanumeric so it never contains a `:`.
pub fn mint_session_cookie(mac: HmacSha256, account_id: &str, ttl_secs: u64) -> String {
    let issued_at = now_unix_secs();
    let expires = issued_at.saturating_add(ttl_secs);
    let payload = format!("v1:{account_id}:{issued_at}:{expires}");
    let sig = sign_session(mac, &payload);
    format!("{payload}:{}", URL_SAFE_NO_PAD.encode(sig))
}

/// Verify a session cookie and, if the signature checks out and it hasn't
/// expired, return the [`SessionClaims`] it carries. Any tampering, malformed
/// structure, or expiry returns `None`.
pub fn verify_session_cookie(mac: HmacSha256, cookie: &str) -> Option<SessionClaims> {
    // v1 : account_id : issued_at : expires : sig  — exactly five fields.
    let mut parts = cookie.splitn(5, ':');
    let (version, account_id, issued, expires, sig) = (
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
    );
    if version != "v1" || account_id.is_empty() {
        return None;
    }
    let issued_at: u64 = issued.parse().ok()?;
    let expires_at: u64 = expires.parse().ok()?;
    if expires_at < now_unix_secs() {
        return None;
    }
    let provided = URL_SAFE_NO_PAD.decode(sig).ok()?;
    let payload = format!("v1:{account_id}:{issued_at}:{expires_at}");
    let expected = sign_session(mac, &payload);
    // Constant-time compare so a byte-by-byte timing side channel can't be
    // used to forge a signature.
    if bool::from(provided.as_slice().ct_eq(expected.as_slice())) {
        Some(SessionClaims {
            account_id: account_id.to_string(),
            issued_at,
        })
    } else {
        None
    }
}

/// Format a `Set-Cookie` header value for a freshly minted session cookie.
/// `secure` gates the `Secure` attribute — true in production (HTTPS via the
/// reverse proxy), false for plain-HTTP localhost dev, where a `Secure`
/// cookie would silently never be sent back.
pub fn session_set_cookie(value: &str, ttl_secs: u64, secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE}={value}; Max-Age={ttl_secs}; Path=/; HttpOnly{secure_attr}; SameSite=Lax"
    )
}

/// Format a `Set-Cookie` header value that immediately clears the session
/// cookie (logout).
pub fn session_clear_cookie(secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!("{SESSION_COOKIE}=; Max-Age=0; Path=/; HttpOnly{secure_attr}; SameSite=Lax")
}

fn sign_session(mut mac: HmacSha256, payload: &str) -> Vec<u8> {
    mac.update(SESSION_SIG_TAG);
    mac.update(payload.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_mac() -> HmacSha256 {
        HmacSha256::new_from_slice(b"test-server-secret").unwrap()
    }

    #[test]
    fn password_hash_roundtrips_and_rejects_wrong() {
        let hash = hash_account_password("correct horse battery staple").unwrap();
        assert!(verify_account_password("correct horse battery staple", &hash));
        assert!(!verify_account_password("wrong password", &hash));
        // Distinct salts → distinct hashes for the same password.
        let hash2 = hash_account_password("correct horse battery staple").unwrap();
        assert_ne!(hash, hash2);
    }

    #[test]
    fn verify_rejects_malformed_stored_hash() {
        assert!(!verify_account_password("anything", "not-a-phc-string"));
        assert!(!verify_account_password("anything", ""));
    }

    #[test]
    fn session_cookie_roundtrips() {
        let cookie = mint_session_cookie(test_mac(), "acct123", 3600);
        let claims = verify_session_cookie(test_mac(), &cookie).expect("valid cookie");
        assert_eq!(claims.account_id, "acct123");
        assert!(claims.issued_at > 0);
    }

    #[test]
    fn session_cookie_rejects_tampering() {
        let cookie = mint_session_cookie(test_mac(), "acct123", 3600);
        // Swap the bound account id but keep the original signature.
        let forged = cookie.replacen("acct123", "attacker", 1);
        assert!(verify_session_cookie(test_mac(), &forged).is_none());
        // A different server secret must not validate a cookie it didn't sign.
        let other_mac = HmacSha256::new_from_slice(b"other-secret").unwrap();
        assert!(verify_session_cookie(other_mac, &cookie).is_none());
    }

    #[test]
    fn session_cookie_rejects_expired() {
        // A cookie that expired back in 1970 (issued_at=1, expires=2).
        let payload = "v1:acct123:1:2";
        let sig = sign_session(test_mac(), payload);
        let expired = format!("{payload}:{}", URL_SAFE_NO_PAD.encode(sig));
        assert!(verify_session_cookie(test_mac(), &expired).is_none());
    }

    #[test]
    fn session_tag_distinct_from_join_token_chain() {
        // Domain separation (MUST-1): a session signature must never equal what
        // the join_token chain (`<name>` then b"join") would produce, even for
        // a crafted account_id. We can't call join_token here, but we assert
        // our tag is a fixed prefix that join_token's input never begins with.
        assert_eq!(SESSION_SIG_TAG, b"session:v1\0");
        assert!(!SESSION_SIG_TAG.starts_with(b"join"));
    }
}
