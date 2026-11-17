//! Fenster verstecken und wieder anzeigen (Hintergrundbetrieb im Tray),
//! abgerundete Fensterecken und Systemschrift.
//!
//! Solange ein Fenster unsichtbar ist, zeichnet eframe keine Frames und
//! verarbeitet deshalb auch keine „zeige dich“-Befehle. Unter Windows wird das
//! Fenster darum direkt über die Win32-API eingeblendet; danach läuft die
//! normale egui-Schleife wieder.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use eframe::egui;

/// Öffnet einen Ordner im Dateimanager des Systems.
pub fn open_folder(path: &std::path::Path) -> std::io::Result<()> {
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(path)
        .spawn()
        .map(|_| ())
}

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn ShowWindow(hwnd: isize, cmd_show: i32) -> i32;
    fn SetForegroundWindow(hwnd: isize) -> i32;
}

#[cfg(windows)]
#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(
        hwnd: isize,
        attribute: u32,
        value: *const std::ffi::c_void,
        size: u32,
    ) -> i32;
}

#[cfg(windows)]
const SW_SHOW: i32 = 5;
/// Windows 11: Eckenform eines Fensters festlegen.
#[cfg(windows)]
const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
#[cfg(windows)]
const DWMWCP_ROUND: i32 = 2;

/// Bittet Windows 11, die Fensterecken abzurunden (das rahmenlose Fenster wäre sonst
/// eckig). Ältere Windows-Versionen ignorieren den Aufruf.
#[cfg(windows)]
fn round_window_corners(hwnd: isize) {
    if hwnd == 0 {
        return;
    }
    let preference = DWMWCP_ROUND;
    // SAFETY: `hwnd` ist unser eigenes Fenster; der Zeiger zeigt auf einen lebenden i32
    // der angegebenen Größe.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &preference as *const i32 as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

/// Thread-sicherer Griff auf das Hauptfenster (auch aus dem Tray-Callback nutzbar).
#[derive(Clone)]
pub struct NativeWindow {
    #[cfg_attr(not(windows), allow(dead_code))]
    hwnd: isize,
    visible: Arc<AtomicBool>,
}

impl NativeWindow {
    pub fn new(cc: &eframe::CreationContext<'_>, visible: bool) -> Self {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let hwnd = match cc.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
            _ => 0,
        };
        #[cfg(windows)]
        round_window_corners(hwnd);

        Self {
            hwnd,
            visible: Arc::new(AtomicBool::new(visible)),
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible.load(Ordering::Relaxed)
    }

    pub fn show(&self, ctx: &egui::Context) {
        self.visible.store(true, Ordering::Relaxed);

        #[cfg(windows)]
        if self.hwnd != 0 {
            // SAFETY: `hwnd` ist das Handle unseres eigenen, lebenden Hauptfensters;
            // beide Funktionen dürfen aus jedem Thread aufgerufen werden.
            unsafe {
                ShowWindow(self.hwnd, SW_SHOW);
                SetForegroundWindow(self.hwnd);
            }
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    pub fn hide(&self, ctx: &egui::Context) {
        self.visible.store(false, Ordering::Relaxed);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }
}

/// Schriftdatei der Windows-Systemschrift „Segoe UI“, falls vorhanden.
pub fn system_font() -> Option<Vec<u8>> {
    if !cfg!(windows) {
        return None;
    }
    let windows_dir = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot"))?;
    std::fs::read(
        std::path::Path::new(&windows_dir)
            .join("Fonts")
            .join("segoeui.ttf"),
    )
    .ok()
}

/// Öffnet eine Webadresse im Standardbrowser. Die Adresse geht als einzelnes Argument
/// direkt an das Programm (keine Shell), Zeichen wie `&` oder `#` bleiben also erhalten.
pub fn open_url(url: &str) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "windows") {
        let mut command = std::process::Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    command.arg(url).spawn().map(|_| ())
}

/// Startet ein Windows-Systemwerkzeug (z. B. `diskmgmt.msc`). Fragt Windows dabei
/// nach Administratorrechten, erscheint die übliche Rückfrage.
#[cfg(windows)]
pub fn run_system_tool(tool: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    /// Kein Konsolenfenster für den kurzlebigen `cmd`-Aufruf anzeigen.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("cmd")
        .args(["/C", "start", "", tool])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

#[cfg(not(windows))]
pub fn run_system_tool(_tool: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Dieses Werkzeug gibt es nur unter Windows.",
    ))
}

// ───────── Ortszeit, Signalton, Bildschirme ─────────

/// Datum und Uhrzeit in der Zeitzone des Systems.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    /// 0 = Sonntag … 6 = Samstag.
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl LocalTime {
    pub fn seconds_of_day(&self) -> i64 {
        self.hour as i64 * 3600 + self.minute as i64 * 60 + self.second as i64
    }
}

fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// Abstand der Ortszeit zu UTC in Sekunden (auf Viertelstunden gerundet).
pub fn local_offset_secs() -> i64 {
    const DAY: i64 = 86_400;
    let mut offset = local_time().seconds_of_day() - unix_seconds().rem_euclid(DAY);
    // Ortszeit und UTC können auf verschiedenen Kalendertagen liegen.
    if offset > 14 * 3600 {
        offset -= DAY;
    } else if offset < -12 * 3600 {
        offset += DAY;
    }
    (offset as f64 / 900.0).round() as i64 * 900
}

#[cfg(windows)]
mod desktop {
    use std::ptr;

    #[repr(C)]
    #[derive(Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }

    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    const HWND_TOPMOST: isize = -1;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    const MB_ICONEXCLAMATION: u32 = 0x30;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(time: *mut SystemTime);
    }

    #[link(name = "user32")]
    extern "system" {
        fn MessageBeep(kind: u32) -> i32;
        fn FindWindowW(class: *const u16, title: *const u16) -> isize;
        fn SetWindowPos(
            window: isize,
            insert_after: isize,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            flags: u32,
        ) -> i32;
        fn SetForegroundWindow(window: isize) -> i32;
        fn GetAsyncKeyState(key: i32) -> i16;
        fn EnumDisplayMonitors(
            dc: isize,
            clip: *const Rect,
            callback: unsafe extern "system" fn(isize, isize, *mut Rect, isize) -> i32,
            data: isize,
        ) -> i32;
    }

    pub fn local_time() -> super::LocalTime {
        let mut time = SystemTime::default();
        // SAFETY: `time` ist eine gültige, beschreibbare SYSTEMTIME-Struktur.
        unsafe { GetLocalTime(&mut time) };
        super::LocalTime {
            year: time.year,
            month: time.month as u8,
            day: time.day as u8,
            weekday: time.day_of_week as u8,
            hour: time.hour as u8,
            minute: time.minute as u8,
            second: time.second as u8,
        }
    }

    pub fn beep() {
        // SAFETY: Aufruf ohne Zeiger.
        unsafe {
            MessageBeep(MB_ICONEXCLAMATION);
        }
    }

    /// Gesamtfläche aller Bildschirme in Pixeln: x, y, Breite, Höhe.
    unsafe extern "system" fn collect(
        _monitor: isize,
        _dc: isize,
        rect: *mut Rect,
        data: isize,
    ) -> i32 {
        // SAFETY: `data` ist der Zeiger auf den Vektor aus `screens`, der während der
        // Aufzählung lebt; `rect` zeigt laut Windows auf das Rechteck des Bildschirms.
        let screens = &mut *(data as *mut Vec<[i32; 4]>);
        if !rect.is_null() {
            let rect = &*rect;
            screens.push([
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
            ]);
        }
        1
    }

    /// Alle Bildschirme in Pixeln: x, y, Breite, Höhe.
    pub fn screens() -> Vec<[i32; 4]> {
        let mut screens: Vec<[i32; 4]> = Vec::new();
        // SAFETY: Der Rückruf erhält den Zeiger auf `screens`, das den Aufruf überlebt.
        unsafe {
            EnumDisplayMonitors(
                0,
                ptr::null(),
                collect,
                &mut screens as *mut Vec<[i32; 4]> as isize,
            );
        }
        screens
    }

    /// Ist diese Taste (Windows-Tastencode, auch Maustasten) gerade gedrückt? Funktioniert auch,
    /// wenn das eigene Fenster keinen Fokus hat.
    pub fn key_down(code: i32) -> bool {
        // SAFETY: Aufruf ohne Zeiger; ungültige Codes liefern 0.
        unsafe { GetAsyncKeyState(code) < 0 }
    }

    /// Ist irgendeine Taste oder Maustaste gedrückt (ohne Umschalttasten)?
    pub fn any_key_down() -> bool {
        // Maustasten 1, 2, 4 sowie Tasten von Rücktaste bis zum Ende; Umschalt-, Strg- und
        // Alt-Tasten (0x10–0x12, 0xA0–0xA5) zählen nicht.
        (0x01..=0x06).any(key_down)
            || (0x08..=0xFE)
                .filter(|code| !matches!(code, 0x10..=0x12 | 0xA0..=0xA5))
                .any(key_down)
    }

    /// Legt das Fenster mit diesem Titel auf das Rechteck `[x, y, Breite, Höhe]` (Pixel) und
    /// holt es nach vorn.
    pub fn place_window(title: &str, rect: [i32; 4], focus: bool) -> bool {
        let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        let [x, y, width, height] = rect;
        // SAFETY: `title` ist nullterminiert; ein unbekanntes Fenster liefert 0.
        unsafe {
            let window = FindWindowW(ptr::null(), title.as_ptr());
            if window == 0 {
                return false;
            }
            SetWindowPos(window, HWND_TOPMOST, x, y, width, height, SWP_SHOWWINDOW);
            if focus {
                SetForegroundWindow(window);
            }
            true
        }
    }
}

#[cfg(not(windows))]
mod desktop {
    pub fn local_time() -> super::LocalTime {
        // Ohne Systemaufruf bleibt nur UTC.
        let seconds = super::unix_seconds().rem_euclid(86_400);
        super::LocalTime {
            hour: (seconds / 3600) as u8,
            minute: (seconds % 3600 / 60) as u8,
            second: (seconds % 60) as u8,
            ..Default::default()
        }
    }

    pub fn beep() {}

    pub fn screens() -> Vec<[i32; 4]> {
        Vec::new()
    }

    pub fn key_down(_code: i32) -> bool {
        false
    }

    pub fn any_key_down() -> bool {
        false
    }

    pub fn place_window(_title: &str, _rect: [i32; 4], _focus: bool) -> bool {
        false
    }
}

pub use desktop::{any_key_down, beep, key_down, local_time, place_window, screens};

/// Kalenderdatum und Uhrzeit eines Zeitstempels (Sekunden seit 1970) in Ortszeit:
/// Jahr, Monat, Tag, Stunde, Minute.
pub fn local_parts(time: u64) -> (i64, i64, i64, i64, i64) {
    let local = time as i64 + local_offset_secs();
    let (days, seconds) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    // Tage seit 1970 → Kalenderdatum (Algorithmus nach Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, seconds / 3600, seconds % 3600 / 60)
}

/// Von RivaTuner Statistics Server (MSI Afterburner) gelesenes Exportsymbol: Existiert es,
/// hängt sich RTSS nicht in diese Anwendung ein. Der Linker-Parameter steht in `build.rs`.
#[no_mangle]
#[used]
#[allow(non_upper_case_globals)]
pub static RTSSHooksCompatibility: u32 = 0;
