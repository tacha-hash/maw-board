//! HTTP and WebSocket handlers for the sshx web interface.

use std::sync::Arc;

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

    Router::new()
        .route("/go", get(go_redirect))
        .nest("/api", backend())
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
    Router::new().route("/s/{name}", any(socket::get_session_ws))
}
