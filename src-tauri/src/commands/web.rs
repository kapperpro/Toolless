//! Web-Integration: EarWeb in einem eigenen, nativen Webview-Fenster.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::error::{AppError, AppResult};

pub const EARWEB_URL: &str = "https://earweb.bttl.xyz/index.html";
const EARWEB_WINDOW: &str = "earweb";

/// Öffnet EarWeb in einem eigenen Fenster (oder fokussiert das bestehende).
///
/// Das Fenster taucht in keiner Capability auf – die fremde Seite hat damit
/// keinerlei Zugriff auf Tauri-Commands oder das Dateisystem.
#[tauri::command]
pub async fn open_earweb_window(app: AppHandle) -> AppResult<()> {
    if let Some(window) = app.get_webview_window(EARWEB_WINDOW) {
        window.unminimize()?;
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }

    let url = EARWEB_URL
        .parse()
        .map_err(|e| AppError::other(format!("Ungültige URL: {e}")))?;
    WebviewWindowBuilder::new(&app, EARWEB_WINDOW, WebviewUrl::External(url))
        .title("EarWeb")
        .inner_size(1100.0, 760.0)
        .min_inner_size(640.0, 480.0)
        .center()
        .build()?;
    Ok(())
}
