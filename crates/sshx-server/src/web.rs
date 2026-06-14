//! HTTP and WebSocket handlers for the sshx web interface.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Form, Query, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, get_service};
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac::{Hmac, Mac as _};
use http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode, Uri};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::ServerState;

pub mod protocol;
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
                let safe = url.replace('"', "%22");
                let html = format!(
                    "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">\
<meta name=\"theme-color\" content=\"#0e0e10\">\
<meta name=\"mobile-web-app-capable\" content=\"yes\">\
<meta name=\"apple-mobile-web-app-capable\" content=\"yes\">\
<title>Oracle Terminal</title>\
<style>html,body{{margin:0;padding:0;width:100%;height:100vh;height:100dvh;\
background:#000;overflow:hidden}}\
iframe{{border:0;width:100%;height:100%;display:block}}</style></head>\
<body><iframe src=\"{safe}\" \
allow=\"microphone; camera; display-capture; clipboard-read; clipboard-write; fullscreen\">\
</iframe></body></html>"
                );
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
    Router::new()
        .route("/s/{name}", any(socket::get_session_ws))
        .route("/sysstat", get(sysstat))
        .route("/files", get(list_files))
        .route("/file", get(read_file))
        .route("/healthz", get(healthz))
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

const BOARD_AUTH_COOKIE: &str = "sshx_board_auth";
const BOARD_AUTH_TTL_SECS: u64 = 60 * 60 * 24 * 30;
type HmacSha256 = Hmac<Sha256>;

#[derive(Deserialize)]
struct LoginParams {
    next: Option<String>,
}

#[derive(Deserialize)]
struct LoginForm {
    password: String,
    next: Option<String>,
}

/// Password middleware for private board routes.
///
/// When `SSHX_BOARD_PASSWORD` is unset, the gate is disabled. When it is set,
/// `/go`, direct `/s/*` session pages, the session WebSocket, and the file APIs
/// require a signed auth cookie minted by the local login form.
pub(crate) async fn board_password_gate(
    State(state): State<Arc<ServerState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let Some(password) = state.board_password() else {
        return next.run(req).await;
    };
    let uri = req.uri().clone();
    let path = uri.path();
    if !requires_board_password(path) || has_valid_board_cookie(req.headers(), password) {
        return next.run(req).await;
    }

    if should_redirect_to_login(req.method(), path) {
        login_redirect(&uri)
    } else {
        (StatusCode::UNAUTHORIZED, "board password required").into_response()
    }
}

async fn login_page(
    State(state): State<Arc<ServerState>>,
    Query(params): Query<LoginParams>,
    headers: HeaderMap,
) -> Response {
    let next = sanitize_next(params.next.as_deref());
    let Some(password) = state.board_password() else {
        return redirect_response(&next, None);
    };
    if has_valid_board_cookie(&headers, password) {
        return redirect_response(&next, None);
    }
    login_form_response(&next, false)
}

async fn login_submit(
    State(state): State<Arc<ServerState>>,
    Form(form): Form<LoginForm>,
) -> Response {
    let next = sanitize_next(form.next.as_deref());
    let Some(password) = state.board_password() else {
        return redirect_response(&next, None);
    };
    if !password_matches(&form.password, password) {
        return login_form_response(&next, true);
    }

    let cookie = make_board_auth_cookie(password);
    redirect_response(&next, Some(&cookie))
}

fn requires_board_password(path: &str) -> bool {
    path == "/go"
        || path == "/s"
        || path.starts_with("/s/")
        || path == "/api/files"
        || path == "/api/file"
        || path == "/api/s"
        || path.starts_with("/api/s/")
}

fn should_redirect_to_login(method: &Method, path: &str) -> bool {
    method == Method::GET && (path == "/go" || path == "/s" || path.starts_with("/s/"))
}

fn login_redirect(uri: &Uri) -> Response {
    let next = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/go");
    let location = format!("/login?next={}", percent_encode_query(next));
    redirect_response(&location, None)
}

fn redirect_response(location: &str, cookie: Option<&str>) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    let location = header_value(location).unwrap_or_else(|| HeaderValue::from_static("/go"));
    response.headers_mut().insert(header::LOCATION, location);
    if let Some(cookie) = cookie {
        let set_cookie = format!(
            "{BOARD_AUTH_COOKIE}={cookie}; Max-Age={BOARD_AUTH_TTL_SECS}; Path=/; HttpOnly; Secure; SameSite=Lax"
        );
        if let Some(value) = header_value(&set_cookie) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}

fn login_form_response(next: &str, failed: bool) -> Response {
    let escaped_next = escape_html_attr(next);
    let message = if failed {
        "<p class=\"error\">Wrong password.</p>"
    } else {
        ""
    };
    let html = format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">\
<meta name=\"theme-color\" content=\"#0e0e10\">\
<title>Oracle Board Login</title>\
<style>html,body{{margin:0;min-height:100%;background:#0e0e10;color:#f5f5f5;\
font-family:system-ui,-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif}}\
body{{display:grid;place-items:center;padding:24px}}main{{width:min(360px,100%)}}\
h1{{font-size:20px;font-weight:650;margin:0 0 16px}}\
form{{display:grid;gap:12px}}input,button{{font:inherit;border-radius:10px}}\
input{{height:44px;border:1px solid #3b3b40;background:#18181b;color:#fff;padding:0 12px}}\
button{{height:46px;border:0;background:#f59e0b;color:#1f1300;font-weight:700}}\
.error{{color:#fca5a5;margin:0 0 12px}}</style></head>\
<body><main><h1>Oracle Board</h1>{message}\
<form method=\"post\" action=\"/login\">\
<input type=\"hidden\" name=\"next\" value=\"{escaped_next}\">\
<input name=\"password\" type=\"password\" autocomplete=\"current-password\" autofocus required>\
<button type=\"submit\">Unlock</button></form></main>\
<script>const n=document.querySelector('input[name=next]');\
if(n&&location.hash&&n.value&&!n.value.includes('#'))n.value+=location.hash;</script>\
</body></html>"
    );
    let status = if failed {
        StatusCode::UNAUTHORIZED
    } else {
        StatusCode::OK
    };
    (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

fn make_board_auth_cookie(password: &str) -> String {
    let expires = now_unix_secs().saturating_add(BOARD_AUTH_TTL_SECS);
    let payload = format!("v1:{expires}");
    let sig = sign_board_auth(password, &payload);
    format!("{payload}:{}", URL_SAFE_NO_PAD.encode(sig))
}

fn has_valid_board_cookie(headers: &HeaderMap, password: &str) -> bool {
    let Some(cookie) = cookie_value(headers, BOARD_AUTH_COOKIE) else {
        return false;
    };
    verify_board_auth_cookie(cookie, password)
}

fn verify_board_auth_cookie(cookie: &str, password: &str) -> bool {
    let mut parts = cookie.split(':');
    let (Some(version), Some(expires), Some(sig), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    if version != "v1" {
        return false;
    }
    let Ok(expires_at) = expires.parse::<u64>() else {
        return false;
    };
    if expires_at < now_unix_secs() {
        return false;
    }
    let Ok(provided) = URL_SAFE_NO_PAD.decode(sig) else {
        return false;
    };
    let payload = format!("v1:{expires_at}");
    let expected = sign_board_auth(password, &payload);
    provided.as_slice().ct_eq(expected.as_slice()).into()
}

fn sign_board_auth(password: &str, payload: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(password.as_bytes()).unwrap();
    mac.update(b"sshx-board-auth-cookie\0");
    mac.update(payload.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn password_matches(submitted: &str, expected: &str) -> bool {
    let submitted_hash = Sha256::digest(submitted.as_bytes());
    let expected_hash = Sha256::digest(expected.as_bytes());
    submitted_hash.ct_eq(&expected_hash).into()
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
        return "/go".to_string();
    };
    if value.starts_with('/')
        && !value.starts_with("//")
        && !value.contains('\r')
        && !value.contains('\n')
    {
        value.to_string()
    } else {
        "/go".to_string()
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

fn escape_html_attr(value: &str) -> String {
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

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

/// Sum of the non-idle and total CPU jiffies from the first line of /proc/stat.
fn read_cpu_jiffies(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().next()?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "cpu" {
        return None;
    }
    let vals: Vec<u64> = parts.filter_map(|v| v.parse().ok()).collect();
    if vals.len() < 4 {
        return None;
    }
    let idle = vals[3] + vals.get(4).copied().unwrap_or(0); // idle + iowait
    let total: u64 = vals.iter().sum();
    Some((idle, total))
}

/// System stats for the workboard status bar: CPU %, RAM, temperature, load.
async fn sysstat() -> Response {
    // CPU %: sample /proc/stat twice ~120ms apart.
    let cpu_pct = async {
        let a = tokio::fs::read_to_string("/proc/stat").await.ok()?;
        let (idle_a, total_a) = read_cpu_jiffies(&a)?;
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        let b = tokio::fs::read_to_string("/proc/stat").await.ok()?;
        let (idle_b, total_b) = read_cpu_jiffies(&b)?;
        let dt = total_b.saturating_sub(total_a);
        let di = idle_b.saturating_sub(idle_a);
        if dt == 0 {
            return None;
        }
        Some(((dt - di) as f64 / dt as f64 * 100.0).round())
    }
    .await;

    // RAM from /proc/meminfo (kB).
    let (mem_total, mem_avail) = {
        let mut total = 0u64;
        let mut avail = 0u64;
        if let Ok(info) = tokio::fs::read_to_string("/proc/meminfo").await {
            for line in info.lines() {
                let mut it = line.split_whitespace();
                match it.next() {
                    Some("MemTotal:") => {
                        total = it.next().and_then(|v| v.parse().ok()).unwrap_or(0)
                    }
                    Some("MemAvailable:") => {
                        avail = it.next().and_then(|v| v.parse().ok()).unwrap_or(0)
                    }
                    _ => {}
                }
            }
        }
        (total, avail)
    };
    let mem_used = mem_total.saturating_sub(mem_avail);
    let mem_pct = if mem_total > 0 {
        (mem_used as f64 / mem_total as f64 * 100.0).round()
    } else {
        0.0
    };

    // Temperature: first readable thermal zone (millidegrees C). May be absent
    // inside an LXC — then null.
    let mut temp: Option<f64> = None;
    for zone in 0..8 {
        let path = format!("/sys/class/thermal/thermal_zone{zone}/temp");
        if let Ok(raw) = tokio::fs::read_to_string(&path).await {
            if let Ok(milli) = raw.trim().parse::<f64>() {
                temp = Some((milli / 1000.0).round());
                break;
            }
        }
    }

    // Load average (1 min).
    let load = tokio::fs::read_to_string("/proc/loadavg")
        .await
        .ok()
        .and_then(|s| s.split_whitespace().next().map(String::from))
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);

    let temp_json = temp.map(|t| t.to_string()).unwrap_or_else(|| "null".into());
    let body = format!(
        "{{\"cpu\":{},\"memUsedMb\":{},\"memTotalMb\":{},\"memPct\":{},\"temp\":{},\"load\":{}}}",
        cpu_pct.unwrap_or(0.0),
        mem_used / 1024,
        mem_total / 1024,
        mem_pct,
        temp_json,
        load,
    );
    ([(http::header::CONTENT_TYPE, "application/json")], body).into_response()
}
