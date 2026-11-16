//! App-Icons aus Verknüpfungen/Programmen lesen (Windows-Shell).
//!
//! Die wenigen benötigten Win32-Funktionen sind hier direkt deklariert, damit
//! kein zusätzliches Binding-Crate nötig ist.

use std::path::Path;

/// Ein Icon als unkomprimierte RGBA-Pixel.
#[derive(Debug, Clone)]
pub struct IconImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Initialisiert COM für den aktuellen Thread, solange der Wert lebt
/// (Voraussetzung für die Shell-Aufrufe).
pub struct ComGuard(());

impl ComGuard {
    pub fn new() -> Self {
        #[cfg(windows)]
        // SAFETY: Standard-Initialisierung; das Gegenstück folgt in `drop`.
        unsafe {
            win::CoInitializeEx(std::ptr::null_mut(), win::COINIT_APARTMENTTHREADED);
        }
        Self(())
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: gleicht genau den Aufruf aus `new` aus.
        unsafe {
            win::CoUninitialize();
        }
    }
}

/// Liest das Icon einer Datei oder Verknüpfung.
#[cfg(windows)]
pub fn extract(path: &Path) -> Option<IconImage> {
    win::extract(path)
}

/// Außerhalb von Windows gibt es (noch) keine Icons – die Oberfläche zeigt dann Initialen.
#[cfg(not(windows))]
pub fn extract(_path: &Path) -> Option<IconImage> {
    None
}

#[cfg(windows)]
mod win {
    use std::{ffi::c_void, mem, os::windows::ffi::OsStrExt, path::Path};

    use super::IconImage;

    pub const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const SHGFI_ICON: u32 = 0x100;
    const SHGFI_LARGEICON: u32 = 0x0;
    const SHGFI_SYSICONINDEX: u32 = 0x4000;
    const ILD_NORMAL: u32 = 0;
    const DIB_RGB_COLORS: u32 = 0;
    const MAX_ICON_SIDE: i32 = 256;

    #[repr(C)]
    struct ShFileInfoW {
        icon: isize,
        icon_index: i32,
        attributes: u32,
        display_name: [u16; 260],
        type_name: [u16; 80],
    }

    #[repr(C)]
    struct IconInfo {
        is_icon: i32,
        x_hotspot: u32,
        y_hotspot: u32,
        mask: isize,
        color: isize,
    }

    #[repr(C)]
    struct Bitmap {
        kind: i32,
        width: i32,
        height: i32,
        width_bytes: i32,
        planes: u16,
        bits_pixel: u16,
        bits: *mut c_void,
    }

    #[repr(C)]
    struct BitmapInfoHeader {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bit_count: u16,
        compression: u32,
        size_image: u32,
        x_pels_per_meter: i32,
        y_pels_per_meter: i32,
        colors_used: u32,
        colors_important: u32,
    }

    #[repr(C)]
    struct BitmapInfo {
        header: BitmapInfoHeader,
        colors: [u32; 3],
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHGetFileInfoW(
            path: *const u16,
            attributes: u32,
            info: *mut ShFileInfoW,
            size: u32,
            flags: u32,
        ) -> usize;
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetIconInfo(icon: isize, info: *mut IconInfo) -> i32;
        fn DestroyIcon(icon: isize) -> i32;
        fn GetDC(window: isize) -> isize;
        fn ReleaseDC(window: isize, dc: isize) -> i32;
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn GetObjectW(object: isize, size: i32, out: *mut c_void) -> i32;
        fn GetDIBits(
            dc: isize,
            bitmap: isize,
            start: u32,
            lines: u32,
            bits: *mut c_void,
            info: *mut BitmapInfo,
            usage: u32,
        ) -> i32;
        fn DeleteObject(object: isize) -> i32;
    }

    #[link(name = "comctl32")]
    extern "system" {
        fn ImageList_GetIcon(image_list: isize, index: i32, flags: u32) -> isize;
    }

    #[link(name = "ole32")]
    extern "system" {
        pub fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> i32;
        pub fn CoUninitialize();
    }

    pub fn extract(path: &Path) -> Option<IconImage> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let info_size = mem::size_of::<ShFileInfoW>() as u32;

        // SAFETY: `wide` ist nullterminiert, `info` ist eine gültige, ausreichend große
        // Struktur. Jedes gelieferte Icon-Handle wird wieder freigegeben; die System-
        // Bildliste gehört Windows und darf nicht freigegeben werden.
        unsafe {
            // Bevorzugt: Icon aus der System-Bildliste. Dort liegt es ohne den
            // Verknüpfungspfeil, den Windows sonst über .lnk-Icons legt.
            let mut info: ShFileInfoW = mem::zeroed();
            let image_list = SHGetFileInfoW(
                wide.as_ptr(),
                0,
                &mut info,
                info_size,
                SHGFI_SYSICONINDEX | SHGFI_LARGEICON,
            );
            if image_list != 0 {
                let icon = ImageList_GetIcon(image_list as isize, info.icon_index, ILD_NORMAL);
                if icon != 0 {
                    let image = icon_to_rgba(icon);
                    DestroyIcon(icon);
                    if image.is_some() {
                        return image;
                    }
                }
            }

            // Rückfall: Icon direkt anfordern (kann den Pfeil enthalten).
            let mut info: ShFileInfoW = mem::zeroed();
            let ok = SHGetFileInfoW(
                wide.as_ptr(),
                0,
                &mut info,
                info_size,
                SHGFI_ICON | SHGFI_LARGEICON,
            );
            if ok == 0 || info.icon == 0 {
                return None;
            }
            let image = icon_to_rgba(info.icon);
            DestroyIcon(info.icon);
            image
        }
    }

    /// Wandelt ein Icon-Handle in RGBA-Pixel um.
    ///
    /// SAFETY: `icon` muss ein gültiges Icon-Handle sein.
    unsafe fn icon_to_rgba(icon: isize) -> Option<IconImage> {
        let mut info: IconInfo = mem::zeroed();
        if GetIconInfo(icon, &mut info) == 0 {
            return None;
        }
        let image = bitmap_to_rgba(info.color);
        // GetIconInfo erzeugt Kopien der Bitmaps, die wir freigeben müssen.
        if info.color != 0 {
            DeleteObject(info.color);
        }
        if info.mask != 0 {
            DeleteObject(info.mask);
        }
        image
    }

    unsafe fn bitmap_to_rgba(bitmap: isize) -> Option<IconImage> {
        if bitmap == 0 {
            return None; // einfarbiges Icon ohne Farb-Bitmap
        }
        let mut meta: Bitmap = mem::zeroed();
        let written = GetObjectW(
            bitmap,
            mem::size_of::<Bitmap>() as i32,
            &mut meta as *mut Bitmap as *mut c_void,
        );
        if written == 0
            || meta.width <= 0
            || meta.height <= 0
            || meta.width > MAX_ICON_SIDE
            || meta.height > MAX_ICON_SIDE
        {
            return None;
        }
        let (width, height) = (meta.width as usize, meta.height as usize);

        let mut bitmap_info: BitmapInfo = mem::zeroed();
        bitmap_info.header.size = mem::size_of::<BitmapInfoHeader>() as u32;
        bitmap_info.header.width = meta.width;
        bitmap_info.header.height = -meta.height; // negativ = Zeilen von oben nach unten
        bitmap_info.header.planes = 1;
        bitmap_info.header.bit_count = 32;

        let mut pixels = vec![0u8; width * height * 4];
        let dc = GetDC(0);
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            pixels.as_mut_ptr() as *mut c_void,
            &mut bitmap_info,
            DIB_RGB_COLORS,
        );
        ReleaseDC(0, dc);
        if lines == 0 {
            return None;
        }

        // BGRA → RGBA. Ältere Icons haben keinen Alphakanal (alles 0) → deckend machen.
        let has_alpha = pixels.chunks_exact(4).any(|px| px[3] != 0);
        for px in pixels.chunks_exact_mut(4) {
            px.swap(0, 2);
            if !has_alpha {
                px[3] = 255;
            }
        }
        Some(IconImage {
            width,
            height,
            rgba: pixels,
        })
    }
}
