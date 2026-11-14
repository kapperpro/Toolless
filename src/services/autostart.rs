//! Autostart-Eintrag im Betriebssystem (Registry / LaunchAgent / XDG-Autostart).

use auto_launch::{AutoLaunch, AutoLaunchBuilder};

use crate::error::{AppError, AppResult};

const APP_NAME: &str = "Toolless";
/// Mit diesem Argument startet die App direkt in den System-Tray.
pub const MINIMIZED_FLAG: &str = "--minimized";

fn launcher() -> AppResult<AutoLaunch> {
    let exe = std::env::current_exe()?;
    let exe = exe
        .to_str()
        .ok_or_else(|| AppError::other("Programmpfad enthält ungültige Zeichen"))?;

    // auto-launch schreibt unter Windows `<pfad> <argumente>` unverändert in die
    // Registry – Pfade mit Leerzeichen müssen deshalb selbst in Anführungszeichen stehen.
    #[cfg(target_os = "windows")]
    let exe = format!("\"{exe}\"");

    let mut builder = AutoLaunchBuilder::new();
    builder
        .set_app_name(APP_NAME)
        .set_app_path(&exe)
        .set_args(&[MINIMIZED_FLAG]);
    #[cfg(target_os = "macos")]
    builder.set_use_launch_agent(true);

    builder
        .build()
        .map_err(|e| AppError::other(format!("Autostart: {e}")))
}

pub fn is_enabled() -> AppResult<bool> {
    launcher()?
        .is_enabled()
        .map_err(|e| AppError::other(format!("Autostart: {e}")))
}

pub fn set(enabled: bool) -> AppResult<bool> {
    let launcher = launcher()?;
    let result = if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    };
    result.map_err(|e| AppError::other(format!("Autostart: {e}")))?;
    Ok(enabled)
}
