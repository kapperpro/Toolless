//! Verschlüsselung der Speicherdateien.
//!
//! Unter Windows über die Data Protection API (DPAPI): Die Daten werden mit einem
//! Schlüssel verschlüsselt, den Windows aus der Anmeldung des Benutzers ableitet.
//! Nur dasselbe Windows-Konto kann die Dateien wieder lesen – ohne zusätzliches
//! Passwort. Auf anderen Systemen gibt es diesen Dienst nicht; dort werden die
//! Dateien unverschlüsselt gespeichert.

use std::{fs, path::Path};

use crate::error::{AppError, AppResult};

/// Kennung am Dateianfang verschlüsselter Dateien.
const MAGIC: &[u8] = b"TOOLLESS-ENC1\n";

/// Schreibt `data` verschlüsselt nach `path` (erst in eine Nachbardatei, dann umbenennen,
/// damit bei einem Absturz nie eine halb geschriebene Datei zurückbleibt).
pub fn write_file(path: &Path, data: &[u8]) -> AppResult<()> {
    let mut contents = Vec::new();
    match protect(data)? {
        Some(encrypted) => {
            contents.extend_from_slice(MAGIC);
            contents.extend_from_slice(&encrypted);
        }
        None => contents.extend_from_slice(data),
    }
    let temp = path.with_extension("tmp");
    fs::write(&temp, contents)?;
    fs::rename(&temp, path)?;
    Ok(())
}

/// Liest eine Datei und entschlüsselt sie. Unverschlüsselte Dateien (ältere
/// Versionen) werden unverändert zurückgegeben.
pub fn read_file(path: &Path) -> AppResult<Vec<u8>> {
    let contents = fs::read(path)?;
    match contents.strip_prefix(MAGIC) {
        Some(encrypted) => unprotect(encrypted),
        None => Ok(contents),
    }
}

/// `Ok(None)`: Auf diesem System steht keine Verschlüsselung zur Verfügung.
#[cfg(not(windows))]
fn protect(_data: &[u8]) -> AppResult<Option<Vec<u8>>> {
    Ok(None)
}

#[cfg(not(windows))]
fn unprotect(_data: &[u8]) -> AppResult<Vec<u8>> {
    Err(AppError::other(
        "Diese Datei wurde unter Windows verschlüsselt und kann hier nicht gelesen werden.",
    ))
}

#[cfg(windows)]
fn protect(data: &[u8]) -> AppResult<Option<Vec<u8>>> {
    dpapi::transform(data, true)
        .map(Some)
        .ok_or_else(|| AppError::other("Verschlüsseln fehlgeschlagen (Windows-Datenschutz)."))
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> AppResult<Vec<u8>> {
    dpapi::transform(data, false).ok_or_else(|| {
        AppError::other("Entschlüsseln fehlgeschlagen – die Datei gehört zu einem anderen Windows-Konto oder ist beschädigt.")
    })
}

#[cfg(windows)]
mod dpapi {
    use std::{ffi::c_void, ptr, slice};

    /// Keine Rückfragen-Dialoge anzeigen.
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    #[repr(C)]
    struct DataBlob {
        len: u32,
        data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            data_in: *const DataBlob,
            description: *const u16,
            entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
        fn CryptUnprotectData(
            data_in: *const DataBlob,
            description: *mut *mut u16,
            entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    /// Ver- (`encrypt = true`) bzw. entschlüsselt `data` für den angemeldeten Benutzer.
    pub fn transform(data: &[u8], encrypt: bool) -> Option<Vec<u8>> {
        let len = u32::try_from(data.len()).ok()?;
        let input = DataBlob {
            len,
            data: data.as_ptr() as *mut u8,
        };
        let mut output = DataBlob {
            len: 0,
            data: ptr::null_mut(),
        };

        // SAFETY: `input` zeigt auf `data`, das während des Aufrufs lebt und nicht verändert
        // wird. `output` wird von Windows gefüllt; der Puffer wird kopiert und danach mit
        // `LocalFree` freigegeben, wie es die API verlangt.
        unsafe {
            let ok = if encrypt {
                CryptProtectData(
                    &input,
                    ptr::null(),
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            } else {
                CryptUnprotectData(
                    &input,
                    ptr::null_mut(),
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            };
            if ok == 0 || output.data.is_null() {
                return None;
            }
            let result = slice::from_raw_parts(output.data, output.len as usize).to_vec();
            LocalFree(output.data as *mut c_void);
            Some(result)
        }
    }
}
