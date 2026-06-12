//! HTTP and WebSocket handlers for the sshx web interface.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::extract::Query;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, get_service};
use axum::Router;
use http::StatusCode;
use tower_http::services::{ServeDir, ServeFile};

use crate::ServerState;

pub mod protocol;
mod socket;

/// Returns the web application server, routed with Axum.
pub fn app() -> Router<Arc<ServerState>> {
    let root_spa = ServeFile::new("build/spa.html")
        .precompressed_gzip()
        .precompressed_br();

    // Serves static SvelteKit build files.
    let static_files = ServeDir::new("build")
        .precompressed_gzip()
        .precompressed_br()
        .fallback(root_spa);

    // Serve hashed build assets WITHOUT the SPA fallback, so a stale client
    // requesting a removed /_app/immutable/* hash gets a 404 (and hard-reloads)
    // instead of SPA HTML served as a JS module (strict-MIME error).
    let app_assets = ServeDir::new("build/_app")
        .precompressed_gzip()
        .precompressed_br();

    Router::new()
        .route("/go", get(go_redirect))
        .nest("/api", backend())
        .nest_service("/_app", get_service(app_assets))
        .fallback_service(get_service(static_files))
}

async fn go_redirect() -> Response {
    match tokio::fs::read_to_string("/root/.sshx-oracle-url.txt").await {
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
<title>Oracle Terminal</title>\
<style>html,body{{margin:0;height:100%;background:#000}}\
iframe{{border:0;width:100%;height:100%;display:block}}</style></head>\
<body><iframe src=\"{safe}\" \
allow=\"microphone; camera; display-capture; clipboard-read; clipboard-write; fullscreen\" \
allowfullscreen></iframe></body></html>"
                );
                ([(http::header::CONTENT_TYPE, "text/html; charset=utf-8")], html)
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
}

/// Read-only file browser root. Listing is confined to this directory; any
/// attempt to escape it (via `..` or absolute components) is rejected.
const FILES_ROOT: &str = "/root/maw-workspace";

/// Resolve a caller-supplied relative path against FILES_ROOT, rejecting any
/// component that could escape the root. Returns None if the path is unsafe.
fn safe_join(rel: &str) -> Option<PathBuf> {
    let root = Path::new(FILES_ROOT);
    let mut out = root.to_path_buf();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            // Reject `..`, absolute, and prefix components outright.
            _ => return None,
        }
    }
    if out.starts_with(root) {
        Some(out)
    } else {
        None
    }
}

/// List a directory under FILES_ROOT for the IDE-style file explorer.
async fn list_files(Query(params): Query<HashMap<String, String>>) -> Response {
    let rel = params.get("path").map(String::as_str).unwrap_or("");
    let Some(dir) = safe_join(rel) else {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
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
    let body = format!("{{\"path\":\"{}\",\"items\":[{}]}}", rel.replace('"', ""), items.join(","));
    (
        [(http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
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
                    Some("MemTotal:") => total = it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
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
    (
        [(http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}
