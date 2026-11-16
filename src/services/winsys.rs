//! Windows-Systemabfragen für den Monitor: Programme mit sichtbarem Fenster,
//! Aufbau der physischen Datenträger (Partitionen).
//!
//! Alles ist nur lesend. Die Win32-Funktionen sind direkt deklariert; die
//! Antwortpuffer der Geräteabfragen werden byteweise an festen, von Windows
//! dokumentierten Positionen gelesen.

#[cfg(not(windows))]
use std::collections::HashSet;

/// Eine Partition (oder ein nicht zugeordneter Bereich) auf einem Datenträger.
#[derive(Debug, Clone, Default)]
pub struct Partition {
    /// Beginn und Größe in Bytes.
    pub offset: u64,
    pub size: u64,
    /// Laufwerksbuchstabe wie `C:`; leer, wenn keiner zugewiesen ist.
    pub letter: String,
    /// Art laut Partitionstabelle, z. B. „EFI-Systempartition“.
    pub kind: String,
    /// `true`: freier, keiner Partition zugeordneter Bereich.
    pub unallocated: bool,
}

/// Ein physischer Datenträger mit seinen Partitionen in Reihenfolge.
#[derive(Debug, Clone, Default)]
pub struct PhysicalDisk {
    pub number: u32,
    pub model: String,
    /// Anschlussart, z. B. „NVMe“, „SATA“, „USB“.
    pub bus: String,
    pub size: u64,
    /// „GPT“ oder „MBR“.
    pub style: String,
    pub removable: bool,
    pub partitions: Vec<Partition>,
}

/// Prozess-IDs aller Programme, die ein sichtbares Hauptfenster haben („Apps“).
#[cfg(not(windows))]
pub fn window_process_ids() -> HashSet<u32> {
    HashSet::new()
}

#[cfg(not(windows))]
pub fn physical_disks() -> Vec<PhysicalDisk> {
    Vec::new()
}

#[cfg(windows)]
pub use win::{physical_disks, window_process_ids};

#[cfg(windows)]
mod win {
    use std::{collections::HashSet, ffi::c_void, ptr};

    use super::{Partition, PhysicalDisk};

    const GW_OWNER: u32 = 4;
    const INVALID_HANDLE: isize = -1;
    const FILE_SHARE_READ_WRITE: u32 = 0x3;
    const OPEN_EXISTING: u32 = 3;
    const IOCTL_DISK_GET_DRIVE_GEOMETRY_EX: u32 = 0x0007_00A0;
    const IOCTL_DISK_GET_DRIVE_LAYOUT_EX: u32 = 0x0007_0050;
    const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
    const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;
    const MAX_DISKS: u32 = 16;
    /// Lücken unter dieser Größe (Ausrichtung der Partitionen) werden nicht angezeigt.
    const MIN_GAP_BYTES: u64 = 8 * 1024 * 1024;

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(callback: extern "system" fn(isize, isize) -> i32, param: isize) -> i32;
        fn IsWindowVisible(window: isize) -> i32;
        fn GetWindowTextLengthW(window: isize) -> i32;
        fn GetWindow(window: isize, command: u32) -> isize;
        fn GetWindowThreadProcessId(window: isize, process_id: *mut u32) -> u32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            disposition: u32,
            flags: u32,
            template: isize,
        ) -> isize;
        fn DeviceIoControl(
            device: isize,
            code: u32,
            input: *const c_void,
            input_size: u32,
            output: *mut c_void,
            output_size: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn CloseHandle(handle: isize) -> i32;
        fn GetLogicalDrives() -> u32;
    }

    // ───────── Fenster → Prozesse ─────────

    extern "system" fn collect_window(window: isize, param: isize) -> i32 {
        // SAFETY: `param` ist der Zeiger auf das `HashSet`, den `window_process_ids` für die
        // Dauer von `EnumWindows` übergibt; die Fensterfunktionen sind für jedes Handle gültig.
        unsafe {
            let is_main_window = IsWindowVisible(window) != 0
                && GetWindowTextLengthW(window) > 0
                && GetWindow(window, GW_OWNER) == 0;
            if is_main_window {
                let mut process_id = 0u32;
                GetWindowThreadProcessId(window, &mut process_id);
                if process_id != 0 {
                    (*(param as *mut HashSet<u32>)).insert(process_id);
                }
            }
        }
        1 // weiter aufzählen
    }

    pub fn window_process_ids() -> HashSet<u32> {
        let mut ids: HashSet<u32> = HashSet::new();
        // SAFETY: Der Rückruf läuft synchron innerhalb von `EnumWindows` und greift nur in
        // dieser Zeit auf `ids` zu.
        unsafe {
            EnumWindows(collect_window, &mut ids as *mut HashSet<u32> as isize);
        }
        ids
    }

    // ───────── Datenträgeraufbau ─────────

    /// Geöffnetes Gerät; wird beim Verwerfen geschlossen.
    struct Device(isize);

    impl Device {
        /// Öffnet ein Gerät nur zum Abfragen (ohne Lese-/Schreibrechte – das geht ohne Adminrechte).
        fn open(path: &str) -> Option<Self> {
            let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
            // SAFETY: `wide` ist nullterminiert; alle weiteren Argumente sind einfache Werte.
            let handle = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    0,
                    FILE_SHARE_READ_WRITE,
                    ptr::null_mut(),
                    OPEN_EXISTING,
                    0,
                    0,
                )
            };
            (handle != INVALID_HANDLE && handle != 0).then_some(Self(handle))
        }

        /// Führt eine Geräteabfrage aus und liefert die Antwortbytes.
        fn query(&self, code: u32, input: &[u8], capacity: usize) -> Option<Vec<u8>> {
            let mut output = vec![0u8; capacity];
            let mut returned = 0u32;
            // SAFETY: Ein- und Ausgabepuffer leben während des Aufrufs und ihre Größen
            // werden korrekt übergeben; `returned` zeigt auf einen lokalen Wert.
            let ok = unsafe {
                DeviceIoControl(
                    self.0,
                    code,
                    if input.is_empty() {
                        ptr::null()
                    } else {
                        input.as_ptr() as *const c_void
                    },
                    input.len() as u32,
                    output.as_mut_ptr() as *mut c_void,
                    capacity as u32,
                    &mut returned,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                return None;
            }
            output.truncate(returned as usize);
            Some(output)
        }
    }

    impl Drop for Device {
        fn drop(&mut self) {
            // SAFETY: Das Handle stammt aus `CreateFileW` und wurde noch nicht geschlossen.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
        Some(u32::from_le_bytes(
            bytes.get(offset..offset + 4)?.try_into().ok()?,
        ))
    }

    fn u64_at(bytes: &[u8], offset: usize) -> Option<u64> {
        Some(u64::from_le_bytes(
            bytes.get(offset..offset + 8)?.try_into().ok()?,
        ))
    }

    /// Nullterminierte ASCII-Zeichenkette ab `offset`.
    fn ascii_at(bytes: &[u8], offset: usize) -> String {
        if offset == 0 || offset >= bytes.len() {
            return String::new();
        }
        let end = bytes[offset..]
            .iter()
            .position(|b| *b == 0)
            .map_or(bytes.len(), |len| offset + len);
        String::from_utf8_lossy(&bytes[offset..end])
            .trim()
            .to_owned()
    }

    fn bus_name(bus: u32) -> &'static str {
        match bus {
            1 => "SCSI",
            2 => "ATAPI",
            3 => "ATA",
            7 => "USB",
            8 => "RAID",
            10 => "SAS",
            11 => "SATA",
            12 => "SD",
            13 => "MMC",
            14 => "Virtuell",
            17 => "NVMe",
            _ => "",
        }
    }

    /// Bekannte GPT-Partitionstypen (GUID in der Byte-Reihenfolge, wie Windows sie liefert).
    fn gpt_kind(guid: &[u8]) -> &'static str {
        const EFI: [u8; 16] = [
            0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E,
            0xC9, 0x3B,
        ];
        const MSR: [u8; 16] = [
            0x16, 0xE3, 0xC9, 0xE3, 0x5C, 0x0B, 0xB8, 0x4D, 0x81, 0x7D, 0xF9, 0x2D, 0xF0, 0x02,
            0x15, 0xAE,
        ];
        const BASIC: [u8; 16] = [
            0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44, 0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26,
            0x99, 0xC7,
        ];
        const RECOVERY: [u8; 16] = [
            0xA4, 0xBB, 0x94, 0xDE, 0xD1, 0x06, 0x40, 0x4D, 0xA1, 0x6A, 0xBF, 0xD5, 0x01, 0x79,
            0xD6, 0xAC,
        ];
        if guid == EFI {
            "EFI-Systempartition"
        } else if guid == MSR {
            "Microsoft-reserviert"
        } else if guid == BASIC {
            "Basisdaten"
        } else if guid == RECOVERY {
            "Wiederherstellung"
        } else {
            "Partition"
        }
    }

    fn mbr_kind(kind: u8) -> &'static str {
        match kind {
            0x07 => "NTFS/exFAT",
            0x0B | 0x0C => "FAT32",
            0x0E | 0x06 => "FAT16",
            0x27 => "Wiederherstellung",
            0x05 | 0x0F => "Erweitert",
            0x83 => "Linux",
            0xEE => "GPT-Schutz",
            _ => "Partition",
        }
    }

    /// Laufwerksbuchstaben mit Datenträgernummer und Startposition ihres Volumes.
    fn volume_extents() -> Vec<(String, u32, u64)> {
        let mut extents = Vec::new();
        // SAFETY: einfache Abfrage ohne Argumente.
        let drives = unsafe { GetLogicalDrives() };
        for index in 0..26u32 {
            if drives & (1 << index) == 0 {
                continue;
            }
            let letter = format!("{}:", (b'A' + index as u8) as char);
            let Some(device) = Device::open(&format!(r"\\.\{letter}")) else {
                continue;
            };
            let Some(bytes) = device.query(IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, &[], 256) else {
                continue;
            };
            // VOLUME_DISK_EXTENTS: Anzahl (4 Bytes, dann Auffüllung), danach je 24 Bytes:
            // Datenträgernummer (4 + 4 Auffüllung), Startposition (8), Länge (8).
            let count = u32_at(&bytes, 0).unwrap_or(0) as usize;
            for extent in 0..count.min(8) {
                let base = 8 + extent * 24;
                if let (Some(disk), Some(offset)) = (u32_at(&bytes, base), u64_at(&bytes, base + 8))
                {
                    extents.push((letter.clone(), disk, offset));
                }
            }
        }
        extents
    }

    fn read_disk(number: u32, extents: &[(String, u32, u64)]) -> Option<PhysicalDisk> {
        let device = Device::open(&format!(r"\\.\PhysicalDrive{number}"))?;

        // DISK_GEOMETRY_EX: Die Gesamtgröße steht hinter der 24 Bytes großen Geometrie.
        let geometry = device.query(IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], 256)?;
        let size = u64_at(&geometry, 24)?;

        // STORAGE_DEVICE_DESCRIPTOR: Wechselmedium @10, Produkt-Offset @16, Anschlussart @28.
        let query = [0u8; 12]; // Eigenschaft „Gerät“, Standardabfrage
        let descriptor = device
            .query(IOCTL_STORAGE_QUERY_PROPERTY, &query, 1024)
            .unwrap_or_default();
        let vendor = ascii_at(&descriptor, u32_at(&descriptor, 12).unwrap_or(0) as usize);
        let product = ascii_at(&descriptor, u32_at(&descriptor, 16).unwrap_or(0) as usize);
        let model = format!("{vendor} {product}").trim().to_owned();
        let removable = descriptor.get(10).is_some_and(|flag| *flag != 0);
        let bus = bus_name(u32_at(&descriptor, 28).unwrap_or(0)).to_owned();

        // DRIVE_LAYOUT_INFORMATION_EX: Stil @0, Anzahl @4, Einträge ab @48 zu je 144 Bytes.
        // Eintrag: Stil @0, Start @8, Länge @16, Nummer @24, Typ ab @32 (GPT: GUID, MBR: 1 Byte).
        let layout = device.query(IOCTL_DISK_GET_DRIVE_LAYOUT_EX, &[], 48 + 144 * 128)?;
        let style = u32_at(&layout, 0)?;
        let count = u32_at(&layout, 4)? as usize;
        let mut partitions = Vec::new();
        for index in 0..count.min(128) {
            let base = 48 + index * 144;
            let (Some(offset), Some(length), Some(partition_number)) = (
                u64_at(&layout, base + 8),
                u64_at(&layout, base + 16),
                u32_at(&layout, base + 24),
            ) else {
                break;
            };
            // Leere Tabelleneinträge (MBR hat immer vier) überspringen.
            if length == 0 || partition_number == 0 {
                continue;
            }
            let kind = if style == 1 {
                layout
                    .get(base + 32..base + 48)
                    .map_or("Partition", gpt_kind)
            } else {
                layout
                    .get(base + 32)
                    .map_or("Partition", |kind| mbr_kind(*kind))
            };
            let letter = extents
                .iter()
                .find(|(_, disk, start)| {
                    *disk == number && *start >= offset && *start < offset + length
                })
                .map(|(letter, _, _)| letter.clone())
                .unwrap_or_default();
            partitions.push(Partition {
                offset,
                size: length,
                letter,
                kind: kind.to_owned(),
                unallocated: false,
            });
        }
        partitions.sort_by_key(|partition| partition.offset);

        // Nicht zugeordnete Bereiche zwischen und hinter den Partitionen ergänzen.
        let mut with_gaps = Vec::new();
        let mut cursor = 0u64;
        for partition in partitions {
            if partition.offset > cursor && partition.offset - cursor >= MIN_GAP_BYTES {
                with_gaps.push(Partition {
                    offset: cursor,
                    size: partition.offset - cursor,
                    unallocated: true,
                    ..Default::default()
                });
            }
            cursor = cursor.max(partition.offset + partition.size);
            with_gaps.push(partition);
        }
        if size > cursor && size - cursor >= MIN_GAP_BYTES {
            with_gaps.push(Partition {
                offset: cursor,
                size: size - cursor,
                unallocated: true,
                ..Default::default()
            });
        }

        Some(PhysicalDisk {
            number,
            model,
            bus,
            size,
            style: match style {
                0 => "MBR",
                1 => "GPT",
                _ => "Unbekannt",
            }
            .to_owned(),
            removable,
            partitions: with_gaps,
        })
    }

    /// Alle physischen Datenträger mit ihrem Partitionsaufbau.
    pub fn physical_disks() -> Vec<PhysicalDisk> {
        let extents = volume_extents();
        (0..MAX_DISKS)
            .filter_map(|number| read_disk(number, &extents))
            .collect()
    }
}
