//! Windows-Leistungsindikatoren („Performance Data Helper“) – dieselbe Quelle,
//! aus der der Task-Manager GPU- und Datenträgeraktivität liest.
//!
//! Ein [`Counter`] steht für einen Indikatorpfad mit Platzhalter, z. B.
//! `\PhysicalDisk(*)\% Disk Time`, und liefert je Instanz einen Wert.

#[cfg(windows)]
pub use win::Counter;

/// Außerhalb von Windows gibt es diese Indikatoren nicht.
#[cfg(not(windows))]
pub struct Counter;

#[cfg(not(windows))]
impl Counter {
    pub fn open(_path: &str) -> Option<Self> {
        None
    }

    pub fn read(&mut self) -> Vec<(String, f64)> {
        Vec::new()
    }
}

#[cfg(windows)]
mod win {
    use std::{ffi::c_void, ptr, slice};

    const PDH_FMT_DOUBLE: u32 = 0x0000_0200;
    const PDH_MORE_DATA: u32 = 0x8000_07D2;
    const MAX_NAME_LEN: usize = 512;

    #[repr(C)]
    struct CounterValue {
        status: u32,
        /// Die Windows-Struktur ist eine Union; wir fordern immer `double` an.
        value: f64,
    }

    #[repr(C)]
    struct CounterItem {
        name: *const u16,
        value: CounterValue,
    }

    #[link(name = "pdh")]
    extern "system" {
        fn PdhOpenQueryW(source: *const u16, user_data: usize, query: *mut isize) -> u32;
        fn PdhAddEnglishCounterW(
            query: isize,
            path: *const u16,
            user_data: usize,
            counter: *mut isize,
        ) -> u32;
        fn PdhCollectQueryData(query: isize) -> u32;
        fn PdhGetFormattedCounterArrayW(
            counter: isize,
            format: u32,
            buffer_size: *mut u32,
            item_count: *mut u32,
            items: *mut c_void,
        ) -> u32;
        fn PdhCloseQuery(query: isize) -> u32;
    }

    pub struct Counter {
        query: isize,
        counter: isize,
    }

    impl Counter {
        /// Öffnet einen Indikator (englischer Pfad, unabhängig von der Systemsprache).
        pub fn open(path: &str) -> Option<Self> {
            let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
            let (mut query, mut counter) = (0isize, 0isize);
            // SAFETY: Alle Zeiger zeigen auf lebende lokale Werte; `path` ist nullterminiert.
            // Schlägt das Hinzufügen fehl, wird die Abfrage wieder geschlossen.
            unsafe {
                if PdhOpenQueryW(ptr::null(), 0, &mut query) != 0 {
                    return None;
                }
                if PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) != 0 {
                    PdhCloseQuery(query);
                    return None;
                }
                // Erste Messung: Grundlage für die Differenz bei der nächsten.
                PdhCollectQueryData(query);
            }
            Some(Self { query, counter })
        }

        /// Aktuelle Werte aller Instanzen (Name, Wert). Leer, solange noch keine
        /// zwei Messungen vorliegen oder etwas schiefgeht.
        pub fn read(&mut self) -> Vec<(String, f64)> {
            let mut values = Vec::new();
            // SAFETY: `query`/`counter` stammen aus `open` und leben so lange wie `self`.
            // Der Puffer ist mindestens so groß, wie Windows es beim ersten Aufruf verlangt,
            // und als `u64` ausgerichtet; Windows legt die Einträge an seinen Anfang und die
            // Namen dahinter. Namen werden höchstens `MAX_NAME_LEN` Zeichen weit gelesen.
            unsafe {
                if PdhCollectQueryData(self.query) != 0 {
                    return values;
                }
                let (mut size, mut count) = (0u32, 0u32);
                let status = PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &mut size,
                    &mut count,
                    ptr::null_mut(),
                );
                if status != PDH_MORE_DATA || size == 0 {
                    return values;
                }
                let mut buffer = vec![0u64; size as usize / 8 + 1];
                let status = PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &mut size,
                    &mut count,
                    buffer.as_mut_ptr() as *mut c_void,
                );
                if status != 0 {
                    return values;
                }

                let items =
                    slice::from_raw_parts(buffer.as_ptr() as *const CounterItem, count as usize);
                for item in items {
                    if item.value.status != 0 || item.name.is_null() {
                        continue;
                    }
                    let mut len = 0;
                    while len < MAX_NAME_LEN && *item.name.add(len) != 0 {
                        len += 1;
                    }
                    let name = String::from_utf16_lossy(slice::from_raw_parts(item.name, len));
                    values.push((name, item.value.value));
                }
            }
            values
        }
    }

    impl Drop for Counter {
        fn drop(&mut self) {
            // SAFETY: `query` ist eine von `open` geöffnete, noch nicht geschlossene Abfrage.
            unsafe {
                PdhCloseQuery(self.query);
            }
        }
    }
}
