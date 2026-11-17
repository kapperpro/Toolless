//! Gesundheits- und Nutzungswerte von Datenträgern (wie CrystalDiskInfo): Modell, Firmware,
//! Seriennummer, Temperatur, Betriebsstunden, gelesene/geschriebene Datenmenge, Fehler und
//! die rohen SMART-Werte. NVMe-Laufwerke werden über die Windows-Speicherschnittstelle
//! gelesen, SATA-Laufwerke über SMART-Befehle (dafür sind meist Administratorrechte nötig).

/// Gesamteindruck wie bei CrystalDiskInfo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Unknown,
    Good,
    Caution,
    Bad,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Unknown => "Unbekannt",
            Status::Good => "Gut",
            Status::Caution => "Achtung",
            Status::Bad => "Schlecht",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SmartInfo {
    pub model: String,
    pub firmware: String,
    pub serial: String,
    /// Schnittstelle, z. B. „NVM Express“, „Serial ATA“, „USB“.
    pub interface: String,
    pub temperature: Option<f32>,
    pub health_percent: Option<u8>,
    pub status: Status,
    /// Gesamte Datenmenge in Bytes.
    pub total_read: Option<u64>,
    pub total_written: Option<u64>,
    pub power_on_hours: Option<u64>,
    pub power_cycles: Option<u64>,
    pub unsafe_shutdowns: Option<u64>,
    /// Medien-/Datenintegritätsfehler bzw. neu zugewiesene Sektoren.
    pub errors: Option<u64>,
    pub error_log_entries: Option<u64>,
    /// Rohwerte: Kennung, Name, Wert (hexadezimal).
    pub rows: Vec<(String, String, String)>,
    /// Hinweis, wenn etwas nicht gelesen werden konnte.
    pub note: Option<String>,
}

fn le(bytes: &[u8], from: usize, len: usize) -> u128 {
    bytes.get(from..from + len).map_or(0, |part| {
        part.iter()
            .rev()
            .fold(0u128, |acc, byte| (acc << 8) | u128::from(*byte))
    })
}

fn clamp_u64(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Wertet die NVMe-Protokollseite „SMART / Health Information“ (512 Bytes) aus.
pub fn parse_nvme_health(page: &[u8], info: &mut SmartInfo) {
    if page.len() < 192 {
        return;
    }
    let critical = page[0];
    let kelvin = le(page, 1, 2) as i32;
    let spare = page[3];
    let spare_threshold = page[4];
    let used = page[5];
    let units = |from| le(page, from, 16);
    let read_units = units(32);
    let written_units = units(48);
    let host_reads = units(64);
    let host_writes = units(80);
    let busy = units(96);
    let cycles = units(112);
    let hours = units(128);
    let unsafe_off = units(144);
    let media_errors = units(160);
    let log_entries = units(176);

    info.temperature = (kelvin > 0).then(|| (kelvin - 273) as f32);
    let health = 100u8.saturating_sub(used.min(100));
    info.health_percent = Some(health);
    info.status = if critical != 0 || health < 10 || spare < spare_threshold {
        Status::Bad
    } else if health < 50 || media_errors > 0 {
        Status::Caution
    } else {
        Status::Good
    };
    // Eine Dateneinheit sind 1000 Blöcke zu 512 Bytes.
    info.total_read = Some(clamp_u64(read_units.saturating_mul(512_000)));
    info.total_written = Some(clamp_u64(written_units.saturating_mul(512_000)));
    info.power_on_hours = Some(clamp_u64(hours));
    info.power_cycles = Some(clamp_u64(cycles));
    info.unsafe_shutdowns = Some(clamp_u64(unsafe_off));
    info.errors = Some(clamp_u64(media_errors));
    info.error_log_entries = Some(clamp_u64(log_entries));

    let raw = |id: &str, name: &str, value: u128| {
        (
            id.to_owned(),
            name.to_owned(),
            format!("{:016X}", clamp_u64(value)),
        )
    };
    info.rows = vec![
        raw("01", "Critical Warning", critical as u128),
        raw("02", "Composite Temperature", kelvin as u128),
        raw("03", "Available Spare", spare as u128),
        raw("04", "Available Spare Threshold", spare_threshold as u128),
        raw("05", "Percentage Used", used as u128),
        raw("06", "Data Units Read", read_units),
        raw("07", "Data Units Written", written_units),
        raw("08", "Host Read Commands", host_reads),
        raw("09", "Host Write Commands", host_writes),
        raw("0A", "Controller Busy Time", busy),
        raw("0B", "Power Cycles", cycles),
        raw("0C", "Power On Hours", hours),
        raw("0D", "Unsafe Shutdowns", unsafe_off),
        raw("0E", "Media and Data Integrity Errors", media_errors),
        raw("0F", "Number of Error Information Log Entries", log_entries),
    ];
}

fn ata_name(id: u8) -> &'static str {
    match id {
        0x01 => "Read Error Rate",
        0x03 => "Spin-Up Time",
        0x04 => "Start/Stop Count",
        0x05 => "Reallocated Sectors Count",
        0x07 => "Seek Error Rate",
        0x09 => "Power-On Hours",
        0x0A => "Spin Retry Count",
        0x0C => "Power Cycle Count",
        0xA1 => "Valid Spare Blocks",
        0xA9 => "Remaining Life",
        0xAA => "Available Reserved Space",
        0xAB => "Program Fail Count",
        0xAC => "Erase Fail Count",
        0xB1 => "Wear Range Delta",
        0xB4 => "Unused Reserved Block Count",
        0xB7 => "SATA Downshift Error Count",
        0xB8 => "End-to-End Error",
        0xBB => "Reported Uncorrectable Errors",
        0xBC => "Command Timeout",
        0xBD => "High Fly Writes",
        0xBE => "Airflow Temperature",
        0xBF => "G-Sense Error Rate",
        0xC0 => "Power-Off Retract Count",
        0xC1 => "Load/Unload Cycle Count",
        0xC2 => "Temperature",
        0xC3 => "Hardware ECC Recovered",
        0xC4 => "Reallocation Event Count",
        0xC5 => "Current Pending Sector Count",
        0xC6 => "Uncorrectable Sector Count",
        0xC7 => "UltraDMA CRC Error Count",
        0xE1 => "Host Writes (32 MiB)",
        0xE7 => "SSD Life Left",
        0xE8 => "Available Reserved Space",
        0xE9 => "Media Wearout Indicator",
        0xF1 => "Total Host Writes",
        0xF2 => "Total Host Reads",
        0xF9 => "Total NAND Writes",
        _ => "Vendor Specific",
    }
}

/// Wertet die 512 Bytes der ATA-Antwort „SMART READ DATA“ aus.
pub fn parse_ata_attributes(data: &[u8], info: &mut SmartInfo) {
    if data.len() < 362 {
        return;
    }
    let mut caution = false;
    for entry in data[2..362].chunks_exact(12) {
        let id = entry[0];
        if id == 0 {
            continue;
        }
        let raw = le(entry, 5, 6);
        info.rows.push((
            format!("{id:02X}"),
            ata_name(id).to_owned(),
            format!("{:012X}", clamp_u64(raw)),
        ));
        let value = clamp_u64(raw);
        match id {
            0x05 | 0xC4 => {
                info.errors = Some(info.errors.unwrap_or(0) + value);
                caution |= value > 0;
            }
            0xC5 | 0xC6 => {
                info.errors = Some(info.errors.unwrap_or(0) + value);
                caution |= value > 0;
            }
            0x09 => info.power_on_hours = Some(value & 0xFFFF_FFFF),
            0x0C => info.power_cycles = Some(value),
            0xC0 | 0xAE => info.unsafe_shutdowns = Some(value),
            0xC2 | 0xBE => {
                // Der erste Byte-Wert ist die aktuelle Temperatur in °C.
                let celsius = (value & 0xFF) as f32;
                if celsius > 0.0 && celsius < 120.0 && (id == 0xC2 || info.temperature.is_none()) {
                    info.temperature = Some(celsius);
                }
            }
            0xF1 => info.total_written = Some(value.saturating_mul(512)),
            0xF2 => info.total_read = Some(value.saturating_mul(512)),
            0xE1 => info.total_written = Some(value.saturating_mul(32 * 1024 * 1024)),
            _ => {}
        }
        // „Normalisierter“ Wert nahe 0 bedeutet bei den Verschleiß-Attributen: fast verbraucht.
        if matches!(id, 0xA9 | 0xE7 | 0xE9 | 0xB1) {
            let current = entry[3];
            if (1..=100).contains(&current) {
                info.health_percent = Some(current);
            }
        }
    }
    info.status = match info.health_percent {
        Some(health) if health < 10 => Status::Bad,
        Some(health) if health < 50 => Status::Caution,
        _ if caution => Status::Caution,
        _ => Status::Good,
    };
}

#[cfg(windows)]
pub fn read(number: u32) -> SmartInfo {
    win::read(number)
}

#[cfg(not(windows))]
pub fn read(_number: u32) -> SmartInfo {
    SmartInfo {
        note: Some("SMART-Werte gibt es nur unter Windows.".to_owned()),
        ..SmartInfo::default()
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    use super::{parse_ata_attributes, parse_nvme_health, SmartInfo};

    type Handle = isize;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            creation: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
        fn DeviceIoControl(
            device: Handle,
            code: u32,
            input: *const c_void,
            input_size: u32,
            output: *mut c_void,
            output_size: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    const INVALID_HANDLE: Handle = -1;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const SHARE_READ_WRITE: u32 = 3;
    const OPEN_EXISTING: u32 = 3;
    const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
    const SMART_RCV_DRIVE_DATA: u32 = 0x0007_C088;

    struct Device(Handle);

    impl Drop for Device {
        fn drop(&mut self) {
            // SAFETY: Das Handle stammt aus `CreateFileW` und wird nur hier geschlossen.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn open(number: u32, access: u32) -> Option<Device> {
        let path: Vec<u16> = format!(r"\\.\PhysicalDrive{number}")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: `path` ist nullterminiert; die übrigen Zeiger sind leer.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                access,
                SHARE_READ_WRITE,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                0,
                0,
            )
        };
        (handle != INVALID_HANDLE).then_some(Device(handle))
    }

    /// Sendet einen Befehl; liefert die Zahl der zurückgegebenen Bytes.
    fn control(device: &Device, code: u32, input: &[u8], output: &mut [u8]) -> Option<usize> {
        let mut returned = 0u32;
        // SAFETY: Beide Puffer sind gültig und ihre Längen werden mitgegeben.
        let ok = unsafe {
            DeviceIoControl(
                device.0,
                code,
                input.as_ptr().cast(),
                input.len() as u32,
                output.as_mut_ptr().cast(),
                output.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(returned as usize)
    }

    fn put(buffer: &mut [u8], at: usize, value: u32) {
        buffer[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn text_at(buffer: &[u8], offset: usize) -> String {
        if offset == 0 || offset >= buffer.len() {
            return String::new();
        }
        let tail = &buffer[offset..];
        let end = tail
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(tail.len());
        String::from_utf8_lossy(&tail[..end]).trim().to_owned()
    }

    /// Modell, Firmware, Seriennummer und Anschlussart.
    fn descriptor(device: &Device, info: &mut SmartInfo) -> Option<u32> {
        let mut query = [0u8; 12];
        put(&mut query, 0, 0); // StorageDeviceProperty
        put(&mut query, 4, 0); // PropertyStandardQuery
        let mut out = vec![0u8; 1024];
        let size = control(device, IOCTL_STORAGE_QUERY_PROPERTY, &query, &mut out)?;
        out.truncate(size);
        if out.len() < 36 {
            return None;
        }
        let word = |at: usize| u32::from_le_bytes([out[at], out[at + 1], out[at + 2], out[at + 3]]);
        let vendor = text_at(&out, word(12) as usize);
        let product = text_at(&out, word(16) as usize);
        info.model = format!("{vendor} {product}").trim().to_owned();
        info.firmware = text_at(&out, word(20) as usize);
        info.serial = text_at(&out, word(24) as usize);
        let bus = word(28);
        info.interface = match bus {
            3 => "ATA",
            7 => "USB",
            8 => "RAID",
            11 => "Serial ATA",
            12 => "SD",
            13 => "MMC",
            17 => "NVM Express",
            _ => "",
        }
        .to_owned();
        Some(bus)
    }

    /// Protokollseite „SMART / Health“ eines NVMe-Laufwerks.
    fn nvme_health(device: &Device) -> Option<Vec<u8>> {
        const PROTOCOL_DATA: usize = 40; // Größe von STORAGE_PROTOCOL_SPECIFIC_DATA
        for sub_value in [0u32, 0xFFFF_FFFF] {
            let mut buffer = vec![0u8; 8 + PROTOCOL_DATA + 512];
            put(&mut buffer, 0, 49); // StorageDeviceProtocolSpecificProperty
            put(&mut buffer, 4, 0); // PropertyStandardQuery
            put(&mut buffer, 8, 3); // ProtocolTypeNvme
            put(&mut buffer, 12, 2); // NVMeDataTypeLogPage
            put(&mut buffer, 16, 2); // Protokollseite 2: SMART / Health
            put(&mut buffer, 20, sub_value);
            put(&mut buffer, 24, PROTOCOL_DATA as u32); // ProtocolDataOffset
            put(&mut buffer, 28, 512); // ProtocolDataLength
            let input = buffer.clone();
            if control(device, IOCTL_STORAGE_QUERY_PROPERTY, &input, &mut buffer).is_some() {
                let offset =
                    8 + u32::from_le_bytes([buffer[24], buffer[25], buffer[26], buffer[27]])
                        as usize;
                if let Some(page) = buffer.get(offset..offset + 512) {
                    return Some(page.to_vec());
                }
            }
        }
        None
    }

    /// 512 Bytes „SMART READ DATA“ eines ATA-Laufwerks (braucht meist Administratorrechte).
    fn ata_smart(device: &Device) -> Option<Vec<u8>> {
        let mut input = [0u8; 32];
        put(&mut input, 0, 512); // cBufferSize
        input[4] = 0xD0; // SMART_READ_DATA
        input[5] = 1;
        input[6] = 1;
        input[7] = 0x4F;
        input[8] = 0xC2;
        input[9] = 0xA0;
        input[10] = 0xB0; // SMART-Befehl
        let mut output = vec![0u8; 16 + 512];
        control(device, SMART_RCV_DRIVE_DATA, &input, &mut output)?;
        Some(output[16..].to_vec())
    }

    pub fn read(number: u32) -> SmartInfo {
        let mut info = SmartInfo::default();
        // Ohne Zugriffsrechte lässt sich das Gerät oft trotzdem abfragen.
        let Some(device) = open(number, 0).or_else(|| open(number, GENERIC_READ | GENERIC_WRITE))
        else {
            info.note = Some("Der Datenträger ließ sich nicht öffnen.".to_owned());
            return info;
        };
        let bus = descriptor(&device, &mut info).unwrap_or(0);

        let mut done = false;
        if bus == 17 || bus == 0 {
            if let Some(page) = nvme_health(&device) {
                parse_nvme_health(&page, &mut info);
                done = true;
            }
        }
        if !done && bus != 17 {
            // Für SMART-Befehle sind Schreibrechte nötig.
            let privileged = open(number, GENERIC_READ | GENERIC_WRITE);
            if let Some(page) = privileged.as_ref().and_then(ata_smart) {
                parse_ata_attributes(&page, &mut info);
                done = true;
            }
        }
        if !done {
            info.note = Some(
                "SMART-Werte konnten nicht gelesen werden. Starte Toolless als Administrator oder prüfe, ob das Laufwerk (z. B. per USB) SMART weitergibt."
                    .to_owned(),
            );
        }
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nvme_health_like_crystaldiskinfo() {
        let mut page = vec![0u8; 512];
        page[1..3].copy_from_slice(&313u16.to_le_bytes()); // 40 °C
        page[3] = 100;
        page[4] = 5;
        page[5] = 18; // 82 % Gesundheit
        page[32..40].copy_from_slice(&1_000u64.to_le_bytes());
        page[48..56].copy_from_slice(&2_000u64.to_le_bytes());
        page[112..120].copy_from_slice(&1248u64.to_le_bytes());
        page[128..136].copy_from_slice(&3341u64.to_le_bytes());
        page[160..168].copy_from_slice(&0u64.to_le_bytes());
        let mut info = SmartInfo::default();
        parse_nvme_health(&page, &mut info);
        assert_eq!(info.temperature, Some(40.0));
        assert_eq!(info.health_percent, Some(82));
        assert_eq!(info.status, Status::Good);
        assert_eq!(info.total_read, Some(512_000_000));
        assert_eq!(info.total_written, Some(1_024_000_000));
        assert_eq!(info.power_cycles, Some(1248));
        assert_eq!(info.power_on_hours, Some(3341));
        assert_eq!(info.rows.len(), 15);
        assert_eq!(info.rows[11].2, "0000000000000D0D");
    }

    #[test]
    fn nvme_with_exhausted_spare_is_bad() {
        let mut page = vec![0u8; 512];
        page[1..3].copy_from_slice(&300u16.to_le_bytes());
        page[3] = 3;
        page[4] = 10;
        let mut info = SmartInfo::default();
        parse_nvme_health(&page, &mut info);
        assert_eq!(info.status, Status::Bad);
    }

    #[test]
    fn parses_ata_attributes() {
        let mut data = vec![0u8; 512];
        let mut put = |slot: usize, id: u8, current: u8, raw: u64| {
            let at = 2 + slot * 12;
            data[at] = id;
            data[at + 3] = current;
            data[at + 5..at + 11].copy_from_slice(&raw.to_le_bytes()[..6]);
        };
        put(0, 0x09, 99, 12_345);
        put(1, 0x0C, 99, 321);
        put(2, 0xC2, 70, 38);
        put(3, 0x05, 100, 4);
        let mut info = SmartInfo::default();
        parse_ata_attributes(&data, &mut info);
        assert_eq!(info.power_on_hours, Some(12_345));
        assert_eq!(info.power_cycles, Some(321));
        assert_eq!(info.temperature, Some(38.0));
        assert_eq!(info.errors, Some(4));
        assert_eq!(info.status, Status::Caution);
        assert_eq!(info.rows.len(), 4);
    }
}
