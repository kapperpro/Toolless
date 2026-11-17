//! Hardware-Angaben für die Seite „Leistung“, die nicht zu den laufenden Messwerten gehören:
//! Cache-Größen, Speichertakt, Netzwerkadresse, Treiberversion usw.
//!
//! Die festen Angaben liest ein einmaliger PowerShell-Aufruf (WMI/CIM) in einem eigenen
//! Thread – das dauert ein bis zwei Sekunden und blockiert die Oberfläche nicht. Die
//! veränderlichen Systemzähler (zugesicherter Speicher, Handles, Threads …) kommen direkt von
//! `GetPerformanceInfo`.

use std::{
    sync::{
        mpsc::{self, Sender},
        Arc, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant},
};

use eframe::egui;
use serde_json::Value;

/// Prozessor.
#[derive(Debug, Clone, Default)]
pub struct CpuInfo {
    pub base_mhz: u64,
    pub sockets: u32,
    pub cores: u32,
    pub logical: u32,
    pub virtualization: Option<bool>,
    pub l1_kb: u64,
    pub l2_kb: u64,
    pub l3_kb: u64,
}

/// Arbeitsspeicher-Module.
#[derive(Debug, Clone, Default)]
pub struct MemoryInfo {
    /// Takt in MT/s.
    pub speed: u64,
    pub slots_used: u32,
    pub slots_total: u32,
    /// „DIMM“, „SODIMM“ …
    pub form_factor: String,
    /// Eingebauter Speicher in Bytes (alle Module).
    pub installed: u64,
}

/// Art eines physischen Datenträgers.
#[derive(Debug, Clone, Default)]
pub struct DiskKind {
    pub number: u32,
    /// „SSD“, „HDD“ oder leer.
    pub media: String,
    /// „SATA“, „NVMe“ …
    pub bus: String,
}

/// Eine verbundene Netzwerkschnittstelle.
#[derive(Debug, Clone, Default)]
pub struct NetworkInfo {
    pub name: String,
    pub description: String,
    pub dns_suffix: String,
    /// „Ethernet“, „WLAN“ …
    pub kind: String,
    pub up: bool,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
}

/// Grafikkarte.
#[derive(Debug, Clone, Default)]
pub struct GpuDetails {
    pub name: String,
    pub driver: String,
    /// Fertig formatiert, z. B. `16.10.2026`.
    pub driver_date: String,
    /// z. B. `PCI bus 3, device 0, function 0`.
    pub location: String,
    /// Dedizierter Grafikspeicher in Bytes (0 = unbekannt).
    pub dedicated: u64,
    /// z. B. `12 (FL 12.2)`.
    pub directx: String,
}

#[derive(Debug, Clone, Default)]
pub struct HwInfo {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub disks: Vec<DiskKind>,
    pub networks: Vec<NetworkInfo>,
    pub gpus: Vec<GpuDetails>,
    /// Laufwerke mit Auslagerungsdatei, z. B. `C:`.
    pub page_files: Vec<String>,
    pub system_drive: String,
    /// Die Grunddaten sind gelesen.
    pub loaded: bool,
}

/// Veränderliche Zähler des Systems.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemStats {
    pub commit_total: u64,
    pub commit_limit: u64,
    /// Zwischengespeicherte Daten (Dateicache) in Bytes.
    pub cached: u64,
    pub paged_pool: u64,
    pub nonpaged_pool: u64,
    pub available: u64,
    /// Für Windows sichtbarer Arbeitsspeicher in Bytes.
    pub visible_total: u64,
    pub handles: u64,
    pub threads: u64,
    pub processes: u64,
    /// Größe des komprimierten Speichers (0 = unbekannt).
    pub compressed: u64,
}

#[cfg(windows)]
pub fn system_stats() -> Option<SystemStats> {
    #[repr(C)]
    struct PerformanceInformation {
        size: u32,
        commit_total: usize,
        commit_limit: usize,
        commit_peak: usize,
        physical_total: usize,
        physical_available: usize,
        system_cache: usize,
        kernel_total: usize,
        kernel_paged: usize,
        kernel_nonpaged: usize,
        page_size: usize,
        handle_count: u32,
        process_count: u32,
        thread_count: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn K32GetPerformanceInfo(info: *mut PerformanceInformation, size: u32) -> i32;
    }

    let mut info = PerformanceInformation {
        size: std::mem::size_of::<PerformanceInformation>() as u32,
        commit_total: 0,
        commit_limit: 0,
        commit_peak: 0,
        physical_total: 0,
        physical_available: 0,
        system_cache: 0,
        kernel_total: 0,
        kernel_paged: 0,
        kernel_nonpaged: 0,
        page_size: 0,
        handle_count: 0,
        process_count: 0,
        thread_count: 0,
    };
    // SAFETY: `info` ist eine lebende, passend aufgebaute Struktur; `size` nennt ihre Größe.
    // Die Funktion schreibt nur in diese Struktur.
    let ok = unsafe { K32GetPerformanceInfo(&mut info, info.size) };
    if ok == 0 || info.page_size == 0 {
        return None;
    }
    let page = info.page_size as u64;
    Some(SystemStats {
        commit_total: info.commit_total as u64 * page,
        commit_limit: info.commit_limit as u64 * page,
        cached: info.system_cache as u64 * page,
        paged_pool: info.kernel_paged as u64 * page,
        nonpaged_pool: info.kernel_nonpaged as u64 * page,
        available: info.physical_available as u64 * page,
        visible_total: info.physical_total as u64 * page,
        handles: u64::from(info.handle_count),
        threads: u64::from(info.thread_count),
        processes: u64::from(info.process_count),
        compressed: 0,
    })
}

#[cfg(not(windows))]
pub fn system_stats() -> Option<SystemStats> {
    None
}

// ───────── PowerShell ─────────

const PS_HEAD: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
"#;

const PS_NET: &str = r#"
$net = @(Get-NetIPConfiguration | Where-Object { $_.IPv4Address -or $_.IPv6Address } | ForEach-Object {
    $a = $_.NetAdapter
    @{
        name = [string]$_.InterfaceAlias
        desc = [string]$_.InterfaceDescription
        up = ([string]$a.Status -eq 'Up')
        media = [string]$a.PhysicalMediaType
        dns = [string](Get-DnsClient -InterfaceIndex $_.InterfaceIndex).ConnectionSpecificSuffix
        v4 = @($_.IPv4Address | ForEach-Object { [string]$_.IPAddress })
        v6 = @($_.IPv6Address | ForEach-Object { [string]$_.IPAddress })
    }
})
"#;

const PS_MAIN: &str = r#"
$cpus = @(Get-CimInstance Win32_Processor)
$c = $cpus | Select-Object -First 1
$l1 = (Get-CimInstance Win32_CacheMemory | Where-Object { $_.Level -eq 3 } | Measure-Object -Property InstalledSize -Sum).Sum
$mods = @(Get-CimInstance Win32_PhysicalMemory)
$m = $mods | Select-Object -First 1
$speed = $m.ConfiguredClockSpeed
if (-not $speed) { $speed = $m.Speed }
$slots = (Get-CimInstance Win32_PhysicalMemoryArray | Measure-Object -Property MemoryDevices -Sum).Sum
$installed = ($mods | Measure-Object -Property Capacity -Sum).Sum
$disks = @(Get-PhysicalDisk | ForEach-Object { @{ n = [int]$_.DeviceId; media = [string]$_.MediaType; bus = [string]$_.BusType } })
$pages = @(Get-CimInstance Win32_PageFileUsage | ForEach-Object { [string]$_.Name })
$cls = 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11cf-bfc1-08002be10318}'
$vram = @{}
Get-ChildItem $cls | ForEach-Object {
    $p = Get-ItemProperty $_.PSPath
    if ($p.DriverDesc) { $vram[[string]$p.DriverDesc] = [int64]$p.'HardwareInformation.qwMemorySize' }
}
$gpus = @(Get-CimInstance Win32_VideoController | ForEach-Object {
    $loc = (Get-PnpDeviceProperty -InstanceId $_.PNPDeviceID -KeyName 'DEVPKEY_Device_LocationInfo').Data
    $date = ''
    if ($_.DriverDate) { $date = $_.DriverDate.ToString('yyyy-MM-dd') }
    @{ name = [string]$_.Name; driver = [string]$_.DriverVersion; date = $date; loc = [string]$loc; mem = [int64]$vram[[string]$_.Name] }
})
$o = @{
    cpu = @{
        base = [int]$c.MaxClockSpeed
        sockets = [int]$cpus.Count
        cores = [int](($cpus | Measure-Object -Property NumberOfCores -Sum).Sum)
        logical = [int](($cpus | Measure-Object -Property NumberOfLogicalProcessors -Sum).Sum)
        virt = [bool]$c.VirtualizationFirmwareEnabled
        l1 = [int64]$l1
        l2 = [int64]$c.L2CacheSize
        l3 = [int64]$c.L3CacheSize
    }
    mem = @{
        speed = [int]$speed
        used = [int]$mods.Count
        total = [int]$slots
        form = [int]$m.FormFactor
        installed = [int64]$installed
    }
    disks = $disks
    pages = $pages
    sysdrive = [string]$env:SystemDrive
    net = $net
    gpus = $gpus
}
$o | ConvertTo-Json -Depth 5 -Compress
"#;

const PS_DIRECTX: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$f = Join-Path $env:TEMP 'toolless-dxdiag.txt'
Remove-Item $f -Force
Start-Process dxdiag -ArgumentList '/whql:off', '/t', $f -WindowStyle Hidden
$last = -1
$same = 0
for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 500
    if (Test-Path $f) {
        $size = (Get-Item $f).Length
        if ($size -eq $last -and $size -gt 0) { $same++ } else { $same = 0 }
        $last = $size
        if ($same -ge 3) { break }
    }
}
Get-Content $f | Select-String 'Card name|DirectX Version|Feature Levels' | ForEach-Object { $_.Line.Trim() }
Remove-Item $f -Force
"#;

fn run_powershell(script: &str) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut command = Command::new("powershell");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Kein Konsolenfenster.
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().ok()?;
    String::from_utf8(output.stdout).ok()
}

// ───────── Auswertung ─────────

static NULL: Value = Value::Null;

fn lock(state: &Mutex<HwInfo>) -> MutexGuard<'_, HwInfo> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn list(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        Value::Null => Vec::new(),
        single => vec![single],
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned()
}

fn number(value: &Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(Value::as_f64)
        .filter(|n| *n > 0.0)
        .map_or(0, |n| n as u64)
}

fn form_factor(code: u64) -> &'static str {
    match code {
        8 => "DIMM",
        12 => "SODIMM",
        _ => "",
    }
}

fn network_kind(media: &str, name: &str) -> String {
    let media = media.to_lowercase();
    let name = name.to_lowercase();
    if media.contains("802.11") || name.contains("wi-fi") || name.contains("wlan") {
        "WLAN".to_owned()
    } else if media.contains("802.3") || name.contains("ethernet") {
        "Ethernet".to_owned()
    } else if media.contains("bluetooth") {
        "Bluetooth".to_owned()
    } else {
        String::new()
    }
}

/// `2026-10-16` → `16.10.2026`.
fn german_date(iso: &str) -> String {
    let mut parts = iso.split('-');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(year), Some(month), Some(day)) if year.len() == 4 => format!("{day}.{month}.{year}"),
        _ => iso.to_owned(),
    }
}

fn parse_networks(root: &Value) -> Vec<NetworkInfo> {
    list(root.get("net").unwrap_or(&NULL))
        .into_iter()
        .map(|net| {
            let name = text(net, "name");
            NetworkInfo {
                kind: network_kind(&text(net, "media"), &name),
                description: text(net, "desc"),
                dns_suffix: text(net, "dns"),
                up: net.get("up").and_then(Value::as_bool).unwrap_or(false),
                ipv4: list(net.get("v4").unwrap_or(&NULL))
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                ipv6: list(net.get("v6").unwrap_or(&NULL))
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                name,
            }
        })
        .collect()
}

/// Wertet die JSON-Ausgabe des Hauptskripts aus.
pub fn parse_main(output: &str) -> Option<HwInfo> {
    let start = output.find('{')?;
    let root: Value = serde_json::from_str(&output[start..]).ok()?;
    let cpu = root.get("cpu").unwrap_or(&NULL);
    let mem = root.get("mem").unwrap_or(&NULL);

    let mut info = HwInfo {
        cpu: CpuInfo {
            base_mhz: number(cpu, "base"),
            sockets: number(cpu, "sockets") as u32,
            cores: number(cpu, "cores") as u32,
            logical: number(cpu, "logical") as u32,
            virtualization: cpu.get("virt").and_then(Value::as_bool),
            l1_kb: number(cpu, "l1"),
            l2_kb: number(cpu, "l2"),
            l3_kb: number(cpu, "l3"),
        },
        memory: MemoryInfo {
            speed: number(mem, "speed"),
            slots_used: number(mem, "used") as u32,
            slots_total: number(mem, "total") as u32,
            form_factor: form_factor(number(mem, "form")).to_owned(),
            installed: number(mem, "installed"),
        },
        networks: parse_networks(&root),
        system_drive: text(&root, "sysdrive"),
        loaded: true,
        ..HwInfo::default()
    };

    info.disks = list(root.get("disks").unwrap_or(&NULL))
        .into_iter()
        .map(|disk| DiskKind {
            number: number(disk, "n") as u32,
            media: match text(disk, "media").as_str() {
                "SSD" => "SSD".to_owned(),
                "HDD" => "HDD".to_owned(),
                _ => String::new(),
            },
            bus: text(disk, "bus"),
        })
        .collect();

    info.page_files = list(root.get("pages").unwrap_or(&NULL))
        .into_iter()
        .filter_map(Value::as_str)
        .filter_map(|path| path.get(..2))
        .map(|letter| letter.to_uppercase())
        .collect();

    info.gpus = list(root.get("gpus").unwrap_or(&NULL))
        .into_iter()
        .map(|gpu| GpuDetails {
            name: text(gpu, "name"),
            driver: text(gpu, "driver"),
            driver_date: german_date(&text(gpu, "date")),
            location: text(gpu, "loc"),
            dedicated: number(gpu, "mem"),
            directx: String::new(),
        })
        .collect();
    Some(info)
}

/// Wertet die Ausgabe von `dxdiag` aus: je Grafikkarte die höchste Feature-Stufe.
/// Ergebnis: (Kartenname, `12 (FL 12.2)`).
pub fn parse_directx(output: &str) -> Vec<(String, String)> {
    let mut version = String::new();
    let mut card = String::new();
    let mut found: Vec<(String, String)> = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("DirectX Version:") {
            version = rest.trim().trim_start_matches("DirectX").trim().to_owned();
        } else if let Some(rest) = line.strip_prefix("Card name:") {
            card = rest.trim().to_owned();
        } else if let Some(rest) = line.strip_prefix("Feature Levels:") {
            let level = rest
                .split(',')
                .next()
                .unwrap_or("")
                .trim()
                .replace('_', ".");
            if !card.is_empty() && !found.iter().any(|(name, _)| *name == card) {
                let text = if level.is_empty() {
                    version.clone()
                } else {
                    format!("{version} (FL {level})")
                };
                found.push((card.clone(), text));
            }
        }
    }
    found
}

// ───────── Zugriff aus der Oberfläche ─────────

/// Liest die Hardware-Angaben im Hintergrund; die Oberfläche holt sich den Stand mit `get`.
pub struct Handle {
    state: Arc<Mutex<HwInfo>>,
    refresh: Sender<()>,
    last_refresh: Mutex<Instant>,
}

/// So selten wird die Netzwerkadresse neu gelesen.
const NETWORK_REFRESH: Duration = Duration::from_secs(20);

impl Handle {
    pub fn spawn(ctx: egui::Context) -> Self {
        let state = Arc::new(Mutex::new(HwInfo::default()));
        let (refresh, requests) = mpsc::channel::<()>();
        let worker_state = Arc::clone(&state);
        let spawned = thread::Builder::new().name("hwinfo".into()).spawn(move || {
            if let Some(info) = run_powershell(&format!("{PS_HEAD}{PS_NET}{PS_MAIN}"))
                .as_deref()
                .and_then(parse_main)
            {
                *lock(&worker_state) = info;
            } else {
                lock(&worker_state).loaded = true;
            }
            ctx.request_repaint();

            // DirectX-Version: dauert einige Sekunden, deshalb getrennt.
            if let Some(output) = run_powershell(PS_DIRECTX) {
                let levels = parse_directx(&output);
                let mut info = lock(&worker_state);
                for gpu in &mut info.gpus {
                    if let Some((_, text)) = levels
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(&gpu.name))
                    {
                        gpu.directx = text.clone();
                    }
                }
                drop(info);
                ctx.request_repaint();
            }

            for () in requests {
                let script = format!(
                    "{PS_HEAD}{PS_NET}\n@{{ net = $net }} | ConvertTo-Json -Depth 5 -Compress"
                );
                let Some(output) = run_powershell(&script) else {
                    continue;
                };
                let Some(start) = output.find('{') else {
                    continue;
                };
                if let Ok(root) = serde_json::from_str::<Value>(&output[start..]) {
                    lock(&worker_state).networks = parse_networks(&root);
                    ctx.request_repaint();
                }
            }
        });
        let _ = spawned;
        Self {
            state,
            refresh,
            last_refresh: Mutex::new(Instant::now()),
        }
    }

    pub fn get(&self) -> HwInfo {
        lock(&self.state).clone()
    }

    /// Fordert eine neue Abfrage der Netzwerkadresse an (höchstens alle 20 Sekunden).
    pub fn refresh_network(&self) {
        let mut last = self
            .last_refresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.elapsed() >= NETWORK_REFRESH {
            *last = Instant::now();
            let _ = self.refresh.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_main_output() {
        let json = r#"{"cpu":{"base":4701,"sockets":1,"cores":6,"logical":12,"virt":true,"l1":384,"l2":6144,"l3":32768},
            "mem":{"speed":4800,"used":2,"total":4,"form":8,"installed":34359738368},
            "disks":{"n":0,"media":"SSD","bus":"SATA"},
            "pages":["C:\\pagefile.sys"],"sysdrive":"C:",
            "net":[{"name":"Ethernet","desc":"Realtek","up":true,"media":"802.3","dns":"home","v4":["192.168.1.120"],"v6":["2a04::1"]}],
            "gpus":[{"name":"AMD Radeon","driver":"32.0.21045.16010","date":"2026-10-16","loc":"PCI bus 3, device 0, function 0","mem":8573157376}]}"#;
        let info = parse_main(json).expect("gültig");
        assert_eq!(info.cpu.base_mhz, 4701);
        assert_eq!(info.cpu.l1_kb, 384);
        assert_eq!(info.cpu.virtualization, Some(true));
        assert_eq!(info.memory.form_factor, "DIMM");
        assert_eq!(info.memory.slots_total, 4);
        assert_eq!(info.disks.len(), 1);
        assert_eq!(info.disks[0].media, "SSD");
        assert_eq!(info.page_files, vec!["C:".to_owned()]);
        assert_eq!(info.networks[0].kind, "Ethernet");
        assert_eq!(info.networks[0].ipv4, vec!["192.168.1.120".to_owned()]);
        assert_eq!(info.gpus[0].driver_date, "16.10.2026");
        assert_eq!(info.gpus[0].dedicated, 8_573_157_376);
    }

    #[test]
    fn ignores_noise_before_json() {
        assert!(parse_main("\u{feff}hallo {\"cpu\":{}}").is_some());
        assert!(parse_main("kein json").is_none());
    }

    #[test]
    fn parses_directx() {
        let text = "DirectX Version: DirectX 12\nCard name: AMD Radeon RX\nFeature Levels: 12_2,12_1,12_0\nCard name: AMD Radeon(TM) Graphics\nFeature Levels: 12_1,12_0";
        let found = parse_directx(text);
        assert_eq!(
            found[0],
            ("AMD Radeon RX".to_owned(), "12 (FL 12.2)".to_owned())
        );
        assert_eq!(found[1].1, "12 (FL 12.1)");
    }
}
