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

/// Shared inline stylesheet for the account auth pages.
const AUTH_STYLE: &str = "html,body{margin:0;min-height:100%;background:#0e0e10;color:#f5f5f5;\
font-family:system-ui,-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif}\
body{display:grid;place-items:center;padding:24px}main{width:min(360px,100%)}\
h1{font-size:20px;font-weight:650;margin:0 0 4px}\
p.sub{margin:0 0 16px;color:#a1a1aa;font-size:14px}\
form{display:grid;gap:12px}label{font-size:13px;color:#a1a1aa;margin:0 0 -6px}\
input,button{font:inherit;border-radius:10px}\
input{height:44px;border:1px solid #3b3b40;background:#18181b;color:#fff;padding:0 12px}\
button{height:46px;border:0;background:#f59e0b;color:#1f1300;font-weight:700;cursor:pointer}\
.error{color:#fca5a5;margin:0 0 12px;font-size:14px}\
.alt{margin:16px 0 0;font-size:14px;color:#a1a1aa}.alt a{color:#f59e0b}";

/// Account login form for `/login` (username + password).
pub fn login_form_page(next: &str, failed: bool) -> String {
    let escaped_next = super::escape_html_attr(next);
    let message = if failed {
        "<p class=\"error\">Incorrect username or password.</p>"
    } else {
        ""
    };
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">\
<meta name=\"theme-color\" content=\"#0e0e10\">\
<title>Oracle Board — Log in</title>\
<style>{AUTH_STYLE}</style></head>\
<body><main><h1>Oracle Board</h1><p class=\"sub\">Log in to your account.</p>{message}\
<form method=\"post\" action=\"/login\">\
<input type=\"hidden\" name=\"next\" value=\"{escaped_next}\">\
<label for=\"u\">Username</label>\
<input id=\"u\" name=\"username\" autocomplete=\"username\" autocapitalize=\"none\" autofocus required>\
<label for=\"p\">Password</label>\
<input id=\"p\" name=\"password\" type=\"password\" autocomplete=\"current-password\" required>\
<button type=\"submit\">Log in</button></form>\
<p class=\"alt\">Have an invite? <a href=\"/join\">Create an account</a>.</p></main>\
</body></html>"
    )
}

/// Account creation form for `/join`, prefilled with the invite `code`.
/// `error` shows a validation/redemption message when the last attempt failed.
pub fn join_form_page(code: &str, error: Option<&str>) -> String {
    let escaped_code = super::escape_html_attr(code);
    let message = match error {
        Some(e) => format!("<p class=\"error\">{}</p>", super::escape_html_attr(e)),
        None => String::new(),
    };
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">\
<meta name=\"theme-color\" content=\"#0e0e10\">\
<title>Oracle Board — Create account</title>\
<style>{AUTH_STYLE}</style></head>\
<body><main><h1>Create your account</h1>\
<p class=\"sub\">You need an invite code to join.</p>{message}\
<form method=\"post\" action=\"/join\">\
<label for=\"c\">Invite code</label>\
<input id=\"c\" name=\"code\" value=\"{escaped_code}\" autocapitalize=\"none\" required>\
<label for=\"u\">Username</label>\
<input id=\"u\" name=\"username\" autocomplete=\"username\" autocapitalize=\"none\" required>\
<label for=\"p\">Password (min 8 characters)</label>\
<input id=\"p\" name=\"password\" type=\"password\" autocomplete=\"new-password\" minlength=\"8\" required>\
<button type=\"submit\">Create account</button></form>\
<p class=\"alt\">Already have an account? <a href=\"/login\">Log in</a>.</p></main>\
</body></html>"
    )
}