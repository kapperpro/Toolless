//! Auswertung der Instanznamen der Windows-Leistungsindikatoren (ohne Systemaufrufe).

use std::collections::HashMap;

/// Zerlegt eine Instanz des Indikators „GPU Engine“, z. B.
/// `pid_1234_luid_0x00000000_0x0000C5F1_phys_0_eng_0_engtype_3D`,
/// in die Kennung der Grafikkarte (LUID) und die Art der Engine.
pub fn gpu_instance(name: &str) -> Option<(u64, &str)> {
    let luid = adapter_luid(name)?;
    let engine = name.split("engtype_").nth(1).unwrap_or("");
    Some((luid, engine))
}

/// LUID aus einem Instanznamen mit `luid_0xHHHHHHHH_0xLLLLLLLL`.
pub fn adapter_luid(name: &str) -> Option<u64> {
    let rest = name.split("luid_").nth(1)?;
    let mut parts = rest.split('_');
    let high = u64::from_str_radix(parts.next()?.trim_start_matches("0x"), 16).ok()?;
    let low = u64::from_str_radix(parts.next()?.trim_start_matches("0x"), 16).ok()?;
    Some((high << 32) | low)
}

/// Auslastung je Grafikkarte wie im Task-Manager: je Engine-Art die Summe über alle
/// Prozesse, davon die höchste.
pub fn gpu_usage(values: &[(String, f64)]) -> HashMap<u64, f32> {
    let mut engines: HashMap<(u64, &str), f64> = HashMap::new();
    for (name, value) in values {
        if let Some(key) = gpu_instance(name) {
            *engines.entry(key).or_default() += value;
        }
    }
    let mut adapters: HashMap<u64, f32> = HashMap::new();
    for ((luid, _), value) in engines {
        let value = value.clamp(0.0, 100.0) as f32;
        let entry = adapters.entry(luid).or_default();
        *entry = entry.max(value);
    }
    adapters
}

/// Summe je Grafikkarte (für Speicher-Indikatoren mit einer Instanz je Karte).
pub fn sum_by_adapter(values: &[(String, f64)]) -> HashMap<u64, f64> {
    let mut adapters: HashMap<u64, f64> = HashMap::new();
    for (name, value) in values {
        if let Some(luid) = adapter_luid(name) {
            *adapters.entry(luid).or_default() += value;
        }
    }
    adapters
}

/// Zerlegt eine Instanz von „PhysicalDisk“, z. B. `1 D: E:`, in Datenträgernummer
/// und Laufwerksbuchstaben. `_Total` und Unbekanntes liefern `None`.
pub fn disk_instance(name: &str) -> Option<(usize, &str)> {
    let name = name.trim();
    let (number, letters) = name.split_once(' ').unwrap_or((name, ""));
    Some((number.parse().ok()?, letters.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(name: &str, value: f64) -> (String, f64) {
        (name.to_owned(), value)
    }

    #[test]
    fn parses_gpu_instances() {
        let name = "pid_1234_luid_0x00000001_0x0000C5F1_phys_0_eng_3_engtype_VideoDecode";
        assert_eq!(gpu_instance(name), Some((0x1_0000_C5F1, "VideoDecode")));
        assert_eq!(
            adapter_luid("luid_0x00000000_0x0000D3A0_phys_0"),
            Some(0xD3A0)
        );
        assert_eq!(gpu_instance("_Total"), None);
        assert_eq!(adapter_luid("luid_kaputt"), None);
    }

    #[test]
    fn aggregates_like_task_manager() {
        let values = [
            value(
                "pid_1_luid_0x00000000_0x0000000A_phys_0_eng_0_engtype_3D",
                30.0,
            ),
            value(
                "pid_2_luid_0x00000000_0x0000000A_phys_0_eng_0_engtype_3D",
                25.0,
            ),
            value(
                "pid_2_luid_0x00000000_0x0000000A_phys_0_eng_1_engtype_Copy",
                70.0,
            ),
            value(
                "pid_3_luid_0x00000000_0x0000000B_phys_0_eng_0_engtype_3D",
                250.0,
            ),
        ];
        let usage = gpu_usage(&values);
        assert_eq!(usage[&0xA], 70.0);
        assert_eq!(usage[&0xB], 100.0);
        let memory = sum_by_adapter(&[
            value("luid_0x00000000_0x0000000A_phys_0", 5.0),
            value("luid_0x00000000_0x0000000A_phys_1", 2.0),
        ]);
        assert_eq!(memory[&0xA], 7.0);
    }

    #[test]
    fn parses_disk_instances() {
        assert_eq!(disk_instance("0 C:"), Some((0, "C:")));
        assert_eq!(disk_instance("1 D: E:"), Some((1, "D: E:")));
        assert_eq!(disk_instance("2"), Some((2, "")));
        assert_eq!(disk_instance("_Total"), None);
    }
}
