//! Oracle Board connector — the menu-bar / system-tray app that brings a
//! machine's agents onto the board. It is a thin supervisor around the
//! connector engine (`oracle-connector`, downloaded on first run): the engine
//! does pairing (opens the browser), discovery, and terminal mirroring; this
//! app gives it a tray icon, a Quit, and launch-on-login so a non-technical
//! user never touches a terminal.

use std::process::{Child, Command};
use std::sync::Mutex;
use std::path::PathBuf;

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, State,
};

const SERVER: &str = "https://board.off-scrn.com";
const RELEASE: &str = "https://github.com/tacha-hash/maw-board/releases/latest/download";

/// The running connector child, so Reconnect/Quit can manage it.
struct ConnectorProc(Mutex<Option<Child>>);

fn oracle_dir() -> PathBuf {
    let d = dirs::home_dir().unwrap_or_default().join(".oracle-connector");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn platform_tag() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", _) => "darwin-x64",
        ("windows", _) => "windows-x64",
        (_, "aarch64") => "linux-arm64",
        _ => "linux-x64",
    }
}

fn connector_path() -> PathBuf {
    let name = if cfg!(windows) { "oracle-connector.exe" } else { "oracle-connector" };
    oracle_dir().join(name)
}

/// Fetch the connector engine for this platform if it isn't already on disk.
fn ensure_connector() -> std::io::Result<PathBuf> {
    let path = connector_path();
    if path.exists() {
        return Ok(path);
    }
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let url = format!("{RELEASE}/oracle-connector-{}{ext}", platform_tag());
    let bytes = reqwest::blocking::get(&url)
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    std::fs::write(&path, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(path)
}

/// Download (if needed) and start the connector. It self-pairs on first run
/// (opens the browser) and reconnects on its own thereafter.
fn spawn_connector() -> Option<Child> {
    let path = ensure_connector().ok()?;
    Command::new(path).arg("--server").arg(SERVER).spawn().ok()
}

fn set_connector(state: &ConnectorProc, child: Option<Child>) {
    let mut guard = state.0.lock().unwrap();
    if let Some(old) = guard.as_mut() {
        let _ = old.kill();
    }
    *guard = child;
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(ConnectorProc(Mutex::new(None)))
        .setup(|app| {
            // Menu-bar app: no Dock icon on macOS, no taskbar churn.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Launch on login (best-effort — user can disable in OS settings).
            {
                use tauri_plugin_autostart::ManagerExt;
                let _ = app.autolaunch().enable();
            }

            // Tray icon + menu.
            let open = MenuItem::with_id(app, "open", "Open Board", true, None::<&str>)?;
            let reconnect = MenuItem::with_id(app, "reconnect", "Reconnect", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &reconnect, &quit])?;
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Oracle Board — your agents on the board")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        use tauri_plugin_opener::OpenerExt;
                        let _ = app.opener().open_url(SERVER, None::<&str>);
                    }
                    "reconnect" => {
                        let state: State<ConnectorProc> = app.state();
                        let fresh = spawn_connector();
                        set_connector(&state, fresh);
                    }
                    "quit" => {
                        let state: State<ConnectorProc> = app.state();
                        set_connector(&state, None); // kills the child
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            // Start the engine. First run downloads it + opens the browser to
            // pair; later runs reconnect silently from its saved config.
            let state: State<ConnectorProc> = app.state();
            let child = spawn_connector();
            set_connector(&state, child);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running oracle-app");
}
