//! Static HTML pages for `/go` and `/login` (extracted from inline strings).

/// Full-page iframe wrapper for `/go` — keeps session URL out of the address bar.
pub fn go_iframe_page(session_url: &str) -> String {
    let safe = session_url.replace('"', "%22");
    format!(
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
    )
}

/// Password gate login form for `/login`.
pub fn login_form_page(next: &str, failed: bool) -> String {
    let escaped_next = super::escape_html_attr(next);
    let message = if failed {
        "<p class=\"error\">Wrong password.</p>"
    } else {
        ""
    };
    format!(
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
    )
}