//! Fenster verstecken und wieder anzeigen (Hintergrundbetrieb im Tray).
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

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn ShowWindow(hwnd: isize, cmd_show: i32) -> i32;
    fn SetForegroundWindow(hwnd: isize) -> i32;
}

#[cfg(windows)]
const SW_SHOW: i32 = 5;

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
