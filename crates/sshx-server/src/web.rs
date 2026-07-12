//! HTTP and WebSocket handlers for the sshx web interface.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Form, Query, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, get_service};
use axum::Router;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode, Uri};
use serde::Deserialize;
use sysinfo::{Components, System};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tracing::error;

use crate::state::accounts::JoinOutcome;
use crate::{auth, ServerState};

pub mod protocol;
mod pages;
mod socket;

/// Returns the web application server, routed with Axum.
pub fn app(state: &ServerState) -> Router<Arc<ServerState>> {
    let static_dir = Path::new(state.static_dir());
    let root_spa = ServeFile::new(static_dir.join("spa.html"))
        .precompressed_gzip()
        .precompressed_br();

    // Serves static SvelteKit build files.
    let static_files = ServeDir::new(static_dir)
        .precompressed_gzip()
        .precompressed_br()
        .fallback(root_spa);

    // Serve hashed build assets WITHOUT the SPA fallback, so a stale client
    // requesting a removed /_app/immutable/* hash gets a 404 (and hard-reloads)
    // instead of SPA HTML served as a JS module (strict-MIME error). These are
    // content-hashed → cache them forever.
    let app_assets = ServeDir::new(static_dir.join("_app"))
        .precompressed_gzip()
        .precompressed_br();

    Router::new()
        .route("/login", get(login_page).post(login_submit))
        .route("/join", get(join_page).post(join_submit))
        .route("/go", get(go_redirect))
        .nest("/api", backend())
        .nest_service(
            "/_app",
            get_service(app_assets).layer(SetResponseHeaderLayer::overriding(
                http::header::CACHE_CONTROL,
                http::HeaderValue::from_static("public, max-age=31536000, immutable"),
            )),
        )
        // Everything else (the SPA HTML) must revalidate so a new deploy is
        // picked up without a manual hard-refresh.
        .fallback_service(
            get_service(static_files).layer(SetResponseHeaderLayer::overriding(
                http::header::CACHE_CONTROL,
                http::HeaderValue::from_static("no-cache"),
            )),
        )
}

async fn go_redirect(State(state): State<Arc<ServerState>>) -> Response {
    match tokio::fs::read_to_string(state.oracle_url_file()).await {
        Ok(contents) => {
            let url = contents.trim();
            if url.is_empty() {
                (StatusCode::SERVICE_UNAVAILABLE, "no active session").into_response()
            } else {
                // Wrap the live session in a full-page iframe so the browser
                // address bar stays at `/go` (the session id + encryption key
                // stay hidden inside the frame instead of redirecting the bar).
                let html = pages::go_iframe_page(url);
                (
                    [(http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    html,
                )
                    .into_response()
            }
        }
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "no active session").into_response(),
    }
}

/// Routes for the backend web API server.
fn backend() -> Router<Arc<ServerState>> {
    use axum::routing::{delete, post};
    Router::new()
        .route("/s/{name}", any(socket::get_session_ws))
        .route("/sysstat", get(sysstat))
        .route("/files", get(list_files))
        .route("/file", get(read_file))
        .route("/logout", post(logout))
        .route("/boards", get(list_boards))
        .route("/boards/new", post(new_board))
        .route("/boards/{name}", delete(delete_board))
        .route("/boards/{name}/key", get(board_key))
        .route("/boards/{name}/members", post(add_member))
        .route("/boards/{name}/members/{username}", delete(remove_member))
        .route("/healthz", get(healthz))
}

/// KDF salt — must match the browser + CLI implementations exactly.
const ENCRYPT_SALT: &str =
    "This is a non-random salt for sshx.io, since we want to stretch the security of 83-bit keys!";

/// Server-side computation of the encrypted-zeros block for a generated key,
/// so empty boards can be created without any client. Mirrors
/// `crates/sshx/src/encrypt.rs` (Argon2id 19MiB/2/1 → AES-128-CTR zero block).
fn encrypted_zeros_for(key: &str) -> Vec<u8> {
    use aes::cipher::{KeyIvInit, StreamCipher};
    use argon2::{Algorithm, Argon2, Params, Version};
    type Aes128Ctr64BE = ctr::Ctr64BE<aes::Aes128>;
    let hasher = Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(19 * 1024, 2, 1, Some(16)).unwrap(),
    );
    let mut aes_key = [0u8; 16];
    hasher
        .hash_password_into(key.as_bytes(), ENCRYPT_SALT.as_bytes(), &mut aes_key)
        .expect("argon2 hashing failed");
    let mut zeros = [0u8; 16];
    let mut cipher = Aes128Ctr64BE::new(&aes_key.into(), &[0u8; 16].into());
    cipher.apply_keystream(&mut zeros);
    zeros.to_vec()
}

/// Create a new empty, persisted board without any backend client.
/// Terminals attach later via multi-backend Join; notes/images/links work
/// immediately. Returns the session name + encryption key so the frontend
/// composes the URL from its own origin.
async fn new_board(
    axum::extract::State(state): axum::extract::State<Arc<ServerState>>,
    account: AuthedAccount,
    body: axum::body::Bytes,
) -> Response {
    use crate::session::{Metadata, Session};
    let parsed: Option<serde_json::Value> = serde_json::from_slice(&body).ok();
    let display_name = parsed
        .as_ref()
        .and_then(|b| b.get("name").and_then(|v| v.as_str()))
        .unwrap_or("board")
        .chars()
        .take(64)
        .collect::<String>();
    let key = sshx_core::rand_alphanumeric(14);
    let name = sshx_core::rand_alphanumeric(10);
    let metadata = Metadata {
        encrypted_zeros: encrypted_zeros_for(&key).into(),
        name: display_name,
        write_password_hash: None,
    };
    let session = Arc::new(Session::new(metadata));
    state.insert(&name, session.clone());
    // Record ownership so the creator (and only they) can delete it, and so it
    // shows up in their per-account lobby. Owner is added as a board member.
    if let Err(err) = state.accounts().create_board(&name, &account.account_id).await {
        error!(?err, "failed to record board ownership");
        return (StatusCode::INTERNAL_SERVER_ERROR, "failed to create board").into_response();
    }
    let jt_for_escrow = crate::grpc::join_token(state.mac(), &name);
    if let Some(disk) = state.disk() {
        if let Ok(snapshot) = session.snapshot() {
            let _ = disk.save(&name, &snapshot);
        }
        // Key escrow for server-created boards (the server generated this key
        // anyway): lets the local bridge daemon auto-join every lobby-created
        // board. 0600, same-user, tailnet-local box — accepted tradeoff.
        let key_path = disk.dir().join(format!("{name}.key"));
        let payload = serde_json::json!({ "key": key, "join_token": jt_for_escrow }).to_string();
        if std::fs::write(&key_path, payload).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
    // join_token ให้ backend (agent host) เข้ามา Join บอร์ดนี้ทีหลังได้ —
    // mint จาก server-wide mac แบบเดียวกับ Open() (tailnet-only จึงยอมรับได้ระดับนี้)
    let jt = crate::grpc::join_token(state.mac(), &name);
    axum::Json(serde_json::json!({ "name": name, "key": key, "join_token": jt })).into_response()
}

/// Permanently close and forget a board (memory + disk snapshot + key escrow +
/// ownership/membership rows). Only the board's owner may delete it.
async fn delete_board(
    axum::extract::State(state): axum::extract::State<Arc<ServerState>>,
    account: AuthedAccount,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    match state.accounts().board_owner(&name).await {
        Ok(Some(owner)) if owner == account.account_id => {}
        // Not the owner, or the board has no ownership record (shouldn't exist
        // post-migration): 404, not 403, so a non-owner can't probe which board
        // names exist.
        Ok(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
        Err(err) => {
            error!(?err, "board_owner lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "error").into_response();
        }
    }
    if let Err(e) = state.close_session(&name).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    if let Err(err) = state.accounts().forget_board(&name).await {
        error!(?err, "failed to forget board rows");
        // The session is already gone; report success on the primary action but
        // log the row-cleanup failure (a stale ownership row is harmless — the
        // board can't be reopened once its snapshot is deleted).
    }
    (StatusCode::OK, "deleted").into_response()
}

/// Lobby listing, scoped to the authenticated account: only boards they own or
/// are a member of. Membership is a live DB query (never cached in the cookie),
/// so revoking a share removes the board from the friend's lobby immediately.
async fn list_boards(
    axum::extract::State(state): axum::extract::State<Arc<ServerState>>,
    account: AuthedAccount,
) -> Response {
    #[derive(serde::Serialize)]
    struct BoardInfo {
        name: String,
        live: bool,
        modified: Option<u64>,
        size: Option<u64>,
    }
    let allowed: std::collections::HashSet<String> = match state
        .accounts()
        .boards_for_account(&account.account_id)
        .await
    {
        Ok(names) => names.into_iter().collect(),
        Err(err) => {
            error!(?err, "boards_for_account failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "error").into_response();
        }
    };
    let live: std::collections::HashSet<String> =
        state.live_session_names().into_iter().collect();
    let mut out: Vec<BoardInfo> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(disk) = state.disk() {
        for b in disk.list() {
            if !allowed.contains(&b.name) {
                continue;
            }
            seen.insert(b.name.clone());
            out.push(BoardInfo {
                live: live.contains(&b.name),
                name: b.name,
                modified: Some(b.modified),
                size: Some(b.size),
            });
        }
    }
    for name in live {
        if allowed.contains(&name) && !seen.contains(&name) {
            out.push(BoardInfo { name, live: true, modified: None, size: None });
        }
    }
    axum::Json(out).into_response()
}

/// Body for adding a member to a board by username.
#[derive(Deserialize)]
struct AddMemberBody {
    username: String,
}

/// Share a board with another account by username (owner only).
async fn add_member(
    axum::extract::State(state): axum::extract::State<Arc<ServerState>>,
    account: AuthedAccount,
    axum::extract::Path(name): axum::extract::Path<String>,
    axum::Json(body): axum::Json<AddMemberBody>,
) -> Response {
    if !require_board_owner(&state, &name, &account.account_id).await {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match state.accounts().add_member_by_username(&name, &body.username).await {
        Ok(true) => (StatusCode::OK, "added").into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "no such user").into_response(),
        Err(err) => {
            error!(?err, "add_member failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "error").into_response()
        }
    }
}

/// Remove a member from a board by username (owner only).
async fn remove_member(
    axum::extract::State(state): axum::extract::State<Arc<ServerState>>,
    account: AuthedAccount,
    axum::extract::Path((name, username)): axum::extract::Path<(String, String)>,
) -> Response {
    if !require_board_owner(&state, &name, &account.account_id).await {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match state.accounts().remove_member_by_username(&name, &username).await {
        Ok(_) => (StatusCode::OK, "removed").into_response(),
        Err(err) => {
            error!(?err, "remove_member failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "error").into_response()
        }
    }
}

/// True only if `account_id` owns board `name`. Used to gate owner-only board
/// administration (delete, membership changes).
async fn require_board_owner(state: &ServerState, name: &str, account_id: &str) -> bool {
    matches!(state.accounts().board_owner(name).await, Ok(Some(owner)) if owner == account_id)
}

/// Board key endpoint (docs/vision-round5-key-via-api-contract.md). Replaces two
/// same-host mechanisms: the browser reading the key from the URL fragment, and
/// the connector reading the escrow `.key` file off a shared disk (which breaks
/// once the server moves to a VPS).
///
/// Members get the read key (plus the write key when the board has one — none do
/// in F0, so `write_key` is null). Connectors (bearer) additionally get a
/// `join_token` so they can attach a backend; browsers deliberately don't —
/// capability separation, so a shared-in member can view/edit but can't spawn a
/// terminal. Both a non-member and a non-existent board return 404, so board
/// names can't be enumerated.
async fn board_key(
    axum::extract::State(state): axum::extract::State<Arc<ServerState>>,
    account: AuthedAccount,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    let not_found =
        || (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({"error": "not found"}))).into_response();

    // Live membership — identical check for browser and connector.
    if !matches!(state.accounts().is_member(&name, &account.account_id).await, Ok(true)) {
        return not_found();
    }

    // The raw key lives only in the server-written escrow file — the Session
    // keeps the derived encrypted-zeros block, not the key itself.
    let Some(disk) = state.disk() else {
        return not_found();
    };
    let escrow_path = disk.dir().join(format!("{name}.key"));
    let key = match std::fs::read_to_string(&escrow_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("key").and_then(|k| k.as_str()).map(str::to_string))
    {
        Some(k) => k,
        None => {
            error!(board = %name, "board key escrow missing/unreadable for a member");
            return (StatusCode::INTERNAL_SERVER_ERROR, "board key unavailable").into_response();
        }
    };

    // write_key is null in F0 (lobby boards carry no write password); emit it
    // explicitly so the bridge's null-tolerant parse is satisfied.
    let mut body = serde_json::json!({ "key": key, "write_key": serde_json::Value::Null });
    if account.is_connector {
        body["join_token"] =
            serde_json::Value::String(crate::grpc::join_token(state.mac(), &name));
    }
    axum::Json(body).into_response()
}

async fn healthz(req: Request<Body>) -> Response {
    let host = req
        .headers()
        .get(http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    let is_local = host.starts_with("localhost")
        || host.starts_with("127.0.0.1")
        || host.starts_with("[::1]")
        || req
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|axum::extract::ConnectInfo(addr)| addr.ip().is_loopback())
            .unwrap_or(false);

    if is_local {
        (StatusCode::OK, "OK").into_response()
    } else {
        (StatusCode::FORBIDDEN, "local-only endpoint").into_response()
    }
}

/// How long a browser session cookie stays valid.
const SESSION_TTL_SECS: u64 = 60 * 60 * 24 * 30;

/// The authenticated identity behind a request, injected by
/// [`session_auth_gate`]. `is_connector` distinguishes a programmatic caller
/// (bearer connector-token) from a browser (session cookie) — the /key endpoint
/// uses it to hand a `join_token` only to connectors (capability separation).
#[derive(Clone)]
struct AuthIdentity {
    account_id: String,
    is_connector: bool,
}

/// An authenticated account, extracted from the identity the
/// [`session_auth_gate`] injected into the request. Handlers that need to know
/// *who* is calling take this; the gate guarantees it's present for every
/// non-public route, so the rejection here is only a defensive fallback.
pub struct AuthedAccount {
    /// The authenticated account's id.
    pub account_id: String,
    /// True when authenticated via a bearer connector-token rather than a
    /// browser session cookie.
    pub is_connector: bool,
}

impl FromRequestParts<Arc<ServerState>> for AuthedAccount {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &Arc<ServerState>,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthIdentity>()
            .map(|i| AuthedAccount {
                account_id: i.account_id.clone(),
                is_connector: i.is_connector,
            })
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, "login required").into_response())
    }
}

/// Authentication + board-access middleware (replaces the old shared-password
/// gate). Every route except the public allow-list requires an authenticated
/// identity — a browser session cookie OR a bearer connector-token; board-scoped
/// routes additionally require live membership. *Authorization* to a board is
/// always a fresh DB query, never trusted from the credential (Le MUST-2).
pub(crate) async fn session_auth_gate(
    State(state): State<Arc<ServerState>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    // Auth pages/APIs and the hashed static bundle are reachable without a
    // session — everything else is gated.
    if is_public_path(&path) {
        return next.run(req).await;
    }

    // Reject cross-origin-sensitive requests from a disallowed Origin (defense
    // in depth alongside the SameSite=Lax cookie).
    if origin_denied(&state, &method, &path, req.headers()) {
        return (StatusCode::FORBIDDEN, "bad origin").into_response();
    }

    // Authentication: who is this? (cookie, else bearer connector-token)
    let Some(identity) = resolve_identity(&state, req.headers()).await else {
        return unauthenticated_response(&method, req.uri());
    };

    // Authorization for board-scoped routes: a live membership check, so
    // removing a share revokes access immediately.
    if let Some(board) = board_scope(&path) {
        if !matches!(
            state.accounts().is_member(board, &identity.account_id).await,
            Ok(true)
        ) {
            return board_forbidden_response(&method, &path);
        }
    }

    req.extensions_mut().insert(identity);
    next.run(req).await
}

/// Resolve the authenticated identity from a request: prefer a browser session
/// cookie, then fall back to an `Authorization: Bearer <connector-token>`. In
/// both cases the account must still exist (deleting an account or rotating its
/// token revokes access). Returns `None` when neither credential validates.
async fn resolve_identity(state: &ServerState, headers: &HeaderMap) -> Option<AuthIdentity> {
    if let Some(claims) = session_from_headers(state, headers) {
        // A cookie for a since-deleted account is not valid.
        return matches!(state.accounts().account_exists(&claims.account_id).await, Ok(true))
            .then_some(AuthIdentity {
                account_id: claims.account_id,
                is_connector: false,
            });
    }
    if let Some(token) = bearer_token(headers) {
        let hash = auth::connector_token_hash(token);
        if let Ok(Some(account_id)) = state.accounts().account_id_by_connector_token(&hash).await {
            return Some(AuthIdentity {
                account_id,
                is_connector: true,
            });
        }
    }
    None
}

/// The bearer token from an `Authorization: Bearer <token>` header, if present.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|t| !t.is_empty())
}

/// Paths reachable without a session: the auth pages/endpoints, logout (a no-op
/// when already logged out), the health probe, the content-hashed app bundle
/// (no data, needed to render the app once logged in), and the board
/// WebSocket.
///
/// The WebSocket (`/api/s/{name}`) is intentionally passed through here in F0
/// chunk 2: it keeps its existing end-to-end encryption-key challenge (a
/// non-member can't obtain the board key — the key is handed out member-only by
/// the /key endpoint), and its session + live-membership + per-shell
/// authorization lands with per-shell ownership enforcement (task 4), where the
/// WS layer gains account awareness. Gating it here would only break the
/// existing WS tests without adding a boundary the key challenge doesn't
/// already provide.
fn is_public_path(path: &str) -> bool {
    matches!(
        path,
        "/login"
            | "/join"
            | "/api/logout"
            | "/api/healthz"
            | "/api/s"
            | "/favicon.ico"
            | "/favicon.png"
            | "/robots.txt"
            | "/manifest.json"
    ) || path.starts_with("/_app/")
        || path.starts_with("/api/s/")
}

/// The board name the board *page* (`/s/{name}`) is scoped to, for the gate's
/// membership check. The WebSocket (`/api/s/{name}`) is handled at the WS layer
/// (task 4), not here.
fn board_scope(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/s/")?;
    let name = rest.split('/').next().unwrap_or(rest);
    (!name.is_empty()).then_some(name)
}

/// Whether to reject a request on Origin grounds. Only checks cross-origin
/// sensitive requests (mutating verbs and the board WS upgrade), only when an
/// allow-list is configured, and only rejects a *present, mismatched* Origin —
/// an absent Origin (CLI tools, the connector) is allowed, since SameSite=Lax
/// already blocks the browser CSRF cases.
fn origin_denied(state: &ServerState, method: &Method, path: &str, headers: &HeaderMap) -> bool {
    let Some(allowed) = state.allowed_origin() else {
        return false;
    };
    let sensitive = !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
        || path.starts_with("/api/s/");
    if !sensitive {
        return false;
    }
    match headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) {
        Some(origin) => origin != allowed,
        None => false,
    }
}

/// Resolve the session claims from a request's cookies, if a valid unexpired
/// cookie is present.
fn session_from_headers(state: &ServerState, headers: &HeaderMap) -> Option<auth::SessionClaims> {
    let cookie = cookie_value(headers, auth::SESSION_COOKIE)?;
    auth::verify_session_cookie(state.mac(), cookie)
}

/// Response for an unauthenticated request: redirect browser GET navigation to
/// the login page (preserving where they were headed), 401 for API/non-GET.
fn unauthenticated_response(method: &Method, uri: &Uri) -> Response {
    if method == Method::GET && !uri.path().starts_with("/api/") {
        login_redirect(uri)
    } else {
        (StatusCode::UNAUTHORIZED, "login required").into_response()
    }
}

/// Response for an authenticated-but-not-a-member request: send browser board
/// navigation back to the lobby, 403 for the WebSocket/API.
fn board_forbidden_response(method: &Method, path: &str) -> Response {
    if method == Method::GET && path.starts_with("/s/") {
        redirect_response("/", None)
    } else {
        (StatusCode::FORBIDDEN, "not a member of this board").into_response()
    }
}

fn login_redirect(uri: &Uri) -> Response {
    let next = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let location = format!("/login?next={}", percent_encode_query(next));
    redirect_response(&location, None)
}

/// A 303 redirect, optionally carrying a full `Set-Cookie` header value.
fn redirect_response(location: &str, set_cookie: Option<&str>) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    let location = header_value(location).unwrap_or_else(|| HeaderValue::from_static("/login"));
    response.headers_mut().insert(header::LOCATION, location);
    if let Some(sc) = set_cookie {
        if let Some(value) = header_value(sc) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}

fn html_page(html: String, status: StatusCode) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

#[derive(Deserialize)]
struct LoginParams {
    next: Option<String>,
}

#[derive(Deserialize)]
struct LoginForm {
    username: String,
    password: String,
    next: Option<String>,
}

async fn login_page(
    State(state): State<Arc<ServerState>>,
    Query(params): Query<LoginParams>,
    headers: HeaderMap,
) -> Response {
    let next = sanitize_next(params.next.as_deref());
    if session_from_headers(&state, &headers).is_some() {
        return redirect_response(&next, None);
    }
    html_page(pages::login_form_page(&next, false), StatusCode::OK)
}

async fn login_submit(
    State(state): State<Arc<ServerState>>,
    Form(form): Form<LoginForm>,
) -> Response {
    let next = sanitize_next(form.next.as_deref());
    let account = state
        .accounts()
        .account_by_username(&form.username)
        .await
        .ok()
        .flatten();
    let verified = match account.as_ref().and_then(|a| a.password_hash.as_deref()) {
        Some(hash) => auth::verify_account_password(&form.password, hash),
        None => {
            // Verify against a throwaway hash even when the username doesn't
            // exist (or is passkey-only), so response time doesn't reveal which
            // usernames are registered.
            let _ = auth::verify_account_password(&form.password, dummy_pw_hash());
            false
        }
    };
    if !verified {
        return html_page(pages::login_form_page(&next, true), StatusCode::UNAUTHORIZED);
    }
    let account = account.expect("verified implies the account was found");
    let value = auth::mint_session_cookie(state.mac(), &account.id, SESSION_TTL_SECS);
    let set_cookie = auth::session_set_cookie(&value, SESSION_TTL_SECS, !state.insecure_cookies());
    redirect_response(&next, Some(&set_cookie))
}

async fn logout(State(state): State<Arc<ServerState>>) -> Response {
    let clear = auth::session_clear_cookie(!state.insecure_cookies());
    let mut response = (StatusCode::OK, "logged out").into_response();
    if let Some(value) = header_value(&clear) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

#[derive(Deserialize)]
struct JoinParams {
    code: Option<String>,
}

#[derive(Deserialize)]
struct JoinForm {
    code: String,
    username: String,
    password: String,
}

async fn join_page(
    State(state): State<Arc<ServerState>>,
    Query(params): Query<JoinParams>,
    headers: HeaderMap,
) -> Response {
    if session_from_headers(&state, &headers).is_some() {
        return redirect_response("/", None);
    }
    let code = params.code.unwrap_or_default();
    html_page(pages::join_form_page(&code, None), StatusCode::OK)
}

async fn join_submit(
    State(state): State<Arc<ServerState>>,
    Form(form): Form<JoinForm>,
) -> Response {
    if !valid_username(&form.username) {
        return html_page(
            pages::join_form_page(
                &form.code,
                Some("Username must be 3–32 characters: letters, digits, - or _."),
            ),
            StatusCode::BAD_REQUEST,
        );
    }
    if form.password.chars().count() < 8 {
        return html_page(
            pages::join_form_page(&form.code, Some("Password must be at least 8 characters.")),
            StatusCode::BAD_REQUEST,
        );
    }
    let hash = match auth::hash_account_password(&form.password) {
        Ok(h) => h,
        Err(err) => {
            error!(?err, "password hashing failed");
            return html_page(
                pages::join_form_page(&form.code, Some("Something went wrong. Please try again.")),
                StatusCode::INTERNAL_SERVER_ERROR,
            );
        }
    };
    match state
        .accounts()
        .redeem_invite(&form.code, &form.username, &hash)
        .await
    {
        Ok(JoinOutcome::Created(account)) => {
            // Auto-login on successful join, straight to the lobby.
            let value = auth::mint_session_cookie(state.mac(), &account.id, SESSION_TTL_SECS);
            let set_cookie =
                auth::session_set_cookie(&value, SESSION_TTL_SECS, !state.insecure_cookies());
            redirect_response("/", Some(&set_cookie))
        }
        Ok(JoinOutcome::InvalidInvite) => html_page(
            pages::join_form_page(&form.code, Some("This invite is invalid or already used.")),
            StatusCode::BAD_REQUEST,
        ),
        Ok(JoinOutcome::UsernameTaken) => html_page(
            pages::join_form_page(&form.code, Some("That username is already taken.")),
            StatusCode::CONFLICT,
        ),
        Err(err) => {
            error!(?err, "invite redemption failed");
            html_page(
                pages::join_form_page(&form.code, Some("Something went wrong. Please try again.")),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        }
    }
}

/// A username is 3–32 chars of `[A-Za-z0-9_-]`. Kept deliberately narrow so
/// usernames are safe to interpolate into HTML/paths and can't collide with the
/// cookie's `:` delimiter.
fn valid_username(u: &str) -> bool {
    let len = u.chars().count();
    (3..=32).contains(&len)
        && u.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A stable throwaway argon2 hash for timing-equalization on failed logins,
/// computed once.
fn dummy_pw_hash() -> &'static str {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DUMMY.get_or_init(|| {
        auth::hash_account_password("timing-equalizer-not-a-real-password").unwrap_or_default()
    })
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    for header in headers.get_all(header::COOKIE) {
        let Ok(value) = header.to_str() else {
            continue;
        };
        for pair in value.split(';') {
            let Some((key, val)) = pair.trim().split_once('=') else {
                continue;
            };
            if key == name {
                return Some(val);
            }
        }
    }
    None
}

fn sanitize_next(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "/".to_string();
    };
    if value.starts_with('/')
        && !value.starts_with("//")
        && !value.contains('\r')
        && !value.contains('\n')
    {
        value.to_string()
    } else {
        "/".to_string()
    }
}

fn percent_encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub(crate) fn escape_html_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn header_value(value: &str) -> Option<HeaderValue> {
    HeaderValue::from_str(value).ok()
}

/// Default file browser root (VPS deployments). Overridable at runtime via the
/// `MAW_FILES_ROOT` env var for hosts without `/root` (e.g. macOS).
const FILES_ROOT_DEFAULT: &str = "/root/maw-workspace";

/// Read-only file browser root. Listing is confined to this directory; any
/// attempt to escape it (via `..` or absolute components) is rejected.
/// Reads `MAW_FILES_ROOT` at call time, falling back to the VPS default so
/// existing deployments are unaffected.
fn files_root() -> PathBuf {
    std::env::var_os("MAW_FILES_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(FILES_ROOT_DEFAULT))
}

/// Resolve a caller-supplied relative path against FILES_ROOT, rejecting any
/// component that could escape the root. Returns None if the path is unsafe.
fn safe_join(rel: &str) -> Option<PathBuf> {
    let root = files_root();
    let mut out = root.clone();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            // Reject `..`, absolute, and prefix components outright.
            _ => return None,
        }
    }
    if out.starts_with(&root) {
        Some(out)
    } else {
        None
    }
}

/// Resolve symlinks and confirm the path stays within FILES_ROOT.
///
/// `safe_join` only inspects path *components* textually — it cannot see a
/// symlink that lives inside the root and points outside it (e.g.
/// `public.txt -> /etc/passwd`). `canonicalize` follows every symlink, so a
/// `starts_with` check on the canonicalized result closes that escape. Returns
/// None on escape OR if the path does not exist (callers map None -> 404, which
/// also avoids leaking which case occurred).
async fn confine_to_root(path: &Path) -> Option<PathBuf> {
    let canon_root = tokio::fs::canonicalize(files_root()).await.ok()?;
    let canon = tokio::fs::canonicalize(path).await.ok()?;
    canon.starts_with(&canon_root).then_some(canon)
}

/// List a directory under FILES_ROOT for the IDE-style file explorer.
async fn list_files(Query(params): Query<HashMap<String, String>>) -> Response {
    let rel = params.get("path").map(String::as_str).unwrap_or("");
    let Some(dir) = safe_join(rel) else {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
    };
    // Resolve symlinks and confirm we stayed inside FILES_ROOT (see confine_to_root).
    let Some(dir) = confine_to_root(&dir).await else {
        return (StatusCode::NOT_FOUND, "not a directory").into_response();
    };
    let mut read = match tokio::fs::read_dir(&dir).await {
        Ok(r) => r,
        Err(_) => return (StatusCode::NOT_FOUND, "not a directory").into_response(),
    };
    let mut entries: Vec<(String, bool, u64)> = Vec::new();
    while let Ok(Some(entry)) = read.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // hide dotfiles (e.g. .git, .ssh-style noise)
        }
        let (is_dir, size) = match entry.metadata().await {
            Ok(m) => (m.is_dir(), m.len()),
            Err(_) => continue,
        };
        entries.push((name, is_dir, size));
    }
    // Directories first, then files, each alphabetical.
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let items: Vec<String> = entries
        .iter()
        .map(|(name, is_dir, size)| {
            format!(
                "{{\"name\":\"{}\",\"dir\":{},\"size\":{}}}",
                name.replace('\\', "\\\\").replace('"', "\\\""),
                is_dir,
                size
            )
        })
        .collect();
    let body = format!(
        "{{\"path\":\"{}\",\"items\":[{}]}}",
        rel.replace('"', ""),
        items.join(",")
    );
    ([(http::header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// Read a text file under FILES_ROOT for the file explorer / viewer.
async fn read_file(Query(params): Query<HashMap<String, String>>) -> Response {
    let rel = params.get("path").map(String::as_str).unwrap_or("");
    let Some(path) = safe_join(rel) else {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
    };

    // SECURITY: Reject dotfiles/dotfolders (components starting with '.')
    for comp in Path::new(rel).components() {
        if let Component::Normal(c) = comp {
            if c.to_string_lossy().starts_with('.') {
                return (StatusCode::BAD_REQUEST, "invalid path").into_response();
            }
        }
    }

    // SECURITY: even inside the shared workspace, never serve credential-bearing
    // files (Bo directive: "watch only for passwords"). Name-based denylist —
    // defense in depth on top of the dedicated, secret-free FILES_ROOT.
    let lower = rel.to_ascii_lowercase();
    // FILES_ROOT is /root/maw-workspace (Bo directive 2026-06-13: "show the old
    // files, just block passwords"). maw-workspace is NOT secret-free, so this
    // name denylist is the primary password/credential guard. SHARED_KNOWLEDGE.md
    // is the fleet's credential/infra index — block it too.
    const CREDENTIAL_MARKERS: [&str; 10] = [
        "secret",
        "credential",
        "password",
        "passwd",
        "token",
        "id_rsa",
        ".key",
        ".pem",
        ".env",
        "shared_knowledge",
    ];
    if CREDENTIAL_MARKERS.iter().any(|m| lower.contains(m)) {
        return (StatusCode::FORBIDDEN, "restricted file").into_response();
    }

    // Resolve symlinks and confirm we stayed inside FILES_ROOT (see confine_to_root).
    // Without this, a symlink inside the workspace (public.txt -> /etc/passwd)
    // escapes the root AND dodges the name-based denylist above.
    let Some(path) = confine_to_root(&path).await else {
        return (StatusCode::NOT_FOUND, "file not found").into_response();
    };

    // Re-apply the credential denylist to the *resolved* path, catching a
    // symlink that points at a credential file elsewhere inside the root.
    let resolved = path.to_string_lossy().to_ascii_lowercase();
    if CREDENTIAL_MARKERS.iter().any(|m| resolved.contains(m)) {
        return (StatusCode::FORBIDDEN, "restricted file").into_response();
    }

    // Check metadata (path is now canonical, so this no longer follows symlinks)
    let metadata = match tokio::fs::metadata(&path).await {
        Ok(m) => m,
        Err(_) => return (StatusCode::NOT_FOUND, "file not found").into_response(),
    };

    if metadata.is_dir() {
        return (StatusCode::BAD_REQUEST, "not a file").into_response();
    }

    // Size limit: 1 MiB (1024 * 1024 bytes) -> 413 Payload Too Large
    if metadata.len() > 1024 * 1024 {
        return (StatusCode::PAYLOAD_TOO_LARGE, "file too large").into_response();
    }

    // Read as string (verifies UTF-8)
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => {
            let escaped_path = escape_json_string(rel);
            let escaped_content = escape_json_string(&content);
            let body = format!(
                "{{\"path\":\"{}\",\"content\":\"{}\"}}",
                escaped_path, escaped_content
            );
            (
                [
                    (http::header::CONTENT_TYPE, "application/json"),
                    (http::header::CACHE_CONTROL, "no-cache"),
                ],
                body,
            )
                .into_response()
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::InvalidData {
                (StatusCode::BAD_REQUEST, "binary file").into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("failed to read file: {e}"),
                )
                    .into_response()
            }
        }
    }
}

/// Helper to escape string characters for valid JSON injection.
fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            _ if c.is_ascii_control() => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            _ => out.push(c),
        }
    }
    out
}

/// System stats for the workboard status bar: CPU %, RAM, temperature, load.
async fn sysstat() -> Response {
    let mut system = System::new();
    system.refresh_memory();

    // CPU usage needs two refreshes separated by sysinfo's minimum sample
    // interval. This replaces the old Linux-only /proc/stat delta.
    system.refresh_cpu_all();
    tokio::time::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;
    system.refresh_cpu_all();
    let cpu_pct = system.global_cpu_usage().round() as f64;

    let mem_total = system.total_memory();
    let mem_used = system.used_memory();
    let mem_pct = if mem_total > 0 {
        (mem_used as f64 / mem_total as f64 * 100.0).round()
    } else {
        0.0
    };

    // Temperature is best-effort. It can be unavailable inside containers and
    // on some macOS hardware, so frontend still receives null in that case.
    let components = Components::new_with_refreshed_list();
    let temp = components
        .iter()
        .filter_map(|component| component.temperature())
        .find(|value| value.is_finite())
        .map(|value| f64::from(value).round());

    let load = System::load_average().one;

    let temp_json = temp.map(|t| t.to_string()).unwrap_or_else(|| "null".into());
    let body = format!(
        "{{\"cpu\":{},\"memUsedMb\":{},\"memTotalMb\":{},\"memPct\":{},\"temp\":{},\"load\":{}}}",
        cpu_pct,
        mem_used / 1024 / 1024,
        mem_total / 1024 / 1024,
        mem_pct,
        temp_json,
        load,
    );
    ([(http::header::CONTENT_TYPE, "application/json")], body).into_response()
}
