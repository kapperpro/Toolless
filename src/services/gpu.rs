//! Grafikkarten: Namen, Temperatur und Leistungsaufnahme.
//!
//! - Namen und Temperatur: für alle Hersteller über den Windows-Grafikkern (dieselbe
//!   Quelle, aus der der Task-Manager die GPU-Temperatur liest).
//! - Gemessene Leistungsaufnahme: NVIDIA über NVML, AMD über die Treiberbibliothek ADL.
//!
//! Die Auslastung kommt aus den Windows-Leistungsindikatoren, siehe `services::monitor`.

/// Eine Grafikkarte, wie Windows sie führt.
#[derive(Debug, Clone, Default)]
pub struct Adapter {
    /// Kennungen, unter denen die Leistungsindikatoren die Karte führen. Windows kann
    /// dieselbe Karte mehrfach melden – solche Einträge sind hier zusammengefasst.
    pub luids: Vec<u64>,
    pub name: String,
    pub temperature: Option<f32>,
}

/// Messwerte der Grafikkarten; nicht verfügbare Werte sind `None`.
#[derive(Debug, Clone, Default)]
pub struct GpuSample {
    /// Alle echten Grafikkarten, nach Namen sortiert (stabil über Neustarts).
    pub adapters: Vec<Adapter>,
    /// Gemessene Leistungsaufnahme aller Karten zusammen in Watt.
    pub power_w: Option<f32>,
}

#[cfg(windows)]
pub use win::GpuProbe;

#[cfg(not(windows))]
pub struct GpuProbe;

#[cfg(not(windows))]
impl GpuProbe {
    pub fn new() -> Self {
        Self
    }

    pub fn sample(&mut self) -> GpuSample {
        GpuSample::default()
    }
}

#[cfg(windows)]
mod win {
    use std::collections::HashMap;

    use nvml_wrapper::Nvml;

    use super::{Adapter, GpuSample};

    pub struct GpuProbe {
        nvml: Option<Nvml>,
        adl: Option<adl::Adl>,
        /// Namen je LUID – sie ändern sich nicht, solange Windows läuft.
        names: HashMap<u64, String>,
    }

    impl GpuProbe {
        pub fn new() -> Self {
            Self {
                // Beides schlägt fehl, wenn der jeweilige Treiber fehlt – dann eben ohne.
                nvml: Nvml::init().ok(),
                adl: adl::Adl::load(),
                names: HashMap::new(),
            }
        }

        pub fn sample(&mut self) -> GpuSample {
            let mut found: Vec<Adapter> = kernel::adapters(&mut self.names);
            found.sort_by(|a, b| a.name.cmp(&b.name).then(a.luids.cmp(&b.luids)));
            // Gleichnamige Einträge sind dieselbe Karte.
            let mut adapters: Vec<Adapter> = Vec::new();
            for adapter in found {
                match adapters.last_mut() {
                    Some(last) if last.name == adapter.name && !adapter.name.is_empty() => {
                        last.luids.extend(adapter.luids);
                        last.temperature = match (last.temperature, adapter.temperature) {
                            (Some(a), Some(b)) => Some(a.max(b)),
                            (a, b) => a.or(b),
                        };
                    }
                    _ => adapters.push(adapter),
                }
            }

            // NVML liefert Milliwatt je Karte.
            let nvidia: Option<f32> = self.nvml.as_ref().and_then(|nvml| {
                let count = nvml.device_count().ok()?;
                let watts: Vec<f32> = (0..count)
                    .filter_map(|index| nvml.device_by_index(index).ok()?.power_usage().ok())
                    .map(|milliwatts| milliwatts as f32 / 1000.0)
                    .collect();
                (!watts.is_empty()).then(|| watts.iter().sum())
            });
            let amd: Option<f32> = self.adl.as_ref().and_then(adl::Adl::power_w);
            let power_w = match (nvidia, amd) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
            };
            GpuSample { adapters, power_w }
        }
    }

    /// AMD Display Library (`atiadlxx.dll`, Teil des Radeon-Treibers): liefert die
    /// Leistungsaufnahme des Grafikchips, wie sie auch die Radeon-Software anzeigt.
    mod adl {
        use std::{collections::HashSet, ffi::c_void, mem, ptr};

        /// Sensor „Leistung des Grafikchips in Watt“ (`ADL_PMLOG_ASIC_POWER`).
        const SENSOR_ASIC_POWER: usize = 23;
        const MAX_SENSORS: usize = 256;
        const MAX_ADAPTERS: usize = 64;
        /// Größe und Feldpositionen von `AdapterInfo` aus dem ADL-SDK (Windows).
        const ADAPTER_INFO_SIZE: usize = 1572;
        const OFFSET_INDEX: usize = 4;
        const OFFSET_BUS: usize = 264;

        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Sensor {
            supported: i32,
            value: i32,
        }

        #[repr(C)]
        struct PmLogData {
            size: i32,
            sensors: [Sensor; MAX_SENSORS],
        }

        type Alloc = unsafe extern "system" fn(i32) -> *mut c_void;
        type Create = unsafe extern "C" fn(Alloc, i32, *mut *mut c_void) -> i32;
        type Destroy = unsafe extern "C" fn(*mut c_void) -> i32;
        type Count = unsafe extern "C" fn(*mut c_void, *mut i32) -> i32;
        type Info = unsafe extern "C" fn(*mut c_void, *mut u8, i32) -> i32;
        type Query = unsafe extern "C" fn(*mut c_void, i32, *mut PmLogData) -> i32;
        type Family = unsafe extern "C" fn(*mut c_void, i32, *mut i32, *mut i32) -> i32;
        /// `ADL_ASIC_INTEGRATED` und `ADL_ASIC_FUSION`: Grafik im Prozessor.
        const ASIC_IN_PROCESSOR: i32 = (1 << 1) | (1 << 5);

        #[link(name = "kernel32")]
        extern "system" {
            fn LoadLibraryW(name: *const u16) -> *mut c_void;
            fn GetProcAddress(module: *mut c_void, name: *const u8) -> *const c_void;
        }

        extern "C" {
            fn malloc(size: usize) -> *mut c_void;
        }

        /// ADL verlangt eine Speicherfunktion für Puffer, die es selbst anlegt.
        unsafe extern "system" fn allocate(size: i32) -> *mut c_void {
            malloc(size.max(0) as usize)
        }

        pub struct Adl {
            context: *mut c_void,
            destroy: Destroy,
            count: Count,
            info: Info,
            query: Query,
            /// Fehlt in sehr alten Treibern.
            family: Option<Family>,
        }

        // SAFETY: Der Kontext wird nur vom Monitor-Thread benutzt, der die Sonde besitzt;
        // ADL2-Kontexte sind nicht an den erzeugenden Thread gebunden.
        unsafe impl Send for Adl {}

        impl Adl {
            pub fn load() -> Option<Self> {
                let name: Vec<u16> = "atiadlxx.dll".encode_utf16().chain(Some(0)).collect();
                // SAFETY: `name` ist nullterminiert. Jede Funktion wird nur verwendet, wenn
                // die Bibliothek sie exportiert; die Signaturen entsprechen dem ADL-SDK.
                unsafe {
                    let module = LoadLibraryW(name.as_ptr());
                    if module.is_null() {
                        return None;
                    }
                    let find = |symbol: &[u8]| {
                        let address = GetProcAddress(module, symbol.as_ptr());
                        (!address.is_null()).then_some(address)
                    };
                    let create: Create = mem::transmute(find(b"ADL2_Main_Control_Create\0")?);
                    let destroy: Destroy = mem::transmute(find(b"ADL2_Main_Control_Destroy\0")?);
                    let count: Count =
                        mem::transmute(find(b"ADL2_Adapter_NumberOfAdapters_Get\0")?);
                    let info: Info = mem::transmute(find(b"ADL2_Adapter_AdapterInfo_Get\0")?);
                    let query: Query = mem::transmute(find(b"ADL2_New_QueryPMLogData_Get\0")?);
                    let family: Option<Family> = find(b"ADL2_Adapter_ASICFamilyType_Get\0")
                        .map(|address| mem::transmute::<*const c_void, Family>(address));

                    let mut context = ptr::null_mut();
                    if create(allocate, 1, &mut context) != 0 || context.is_null() {
                        return None;
                    }
                    Some(Self {
                        context,
                        destroy,
                        count,
                        info,
                        query,
                        family,
                    })
                }
            }

            /// Summe der Chipleistung aller AMD-Karten; `None`, wenn keine sie meldet.
            pub fn power_w(&self) -> Option<f32> {
                // SAFETY: Der Kontext stammt aus `load`. Der Infopuffer ist genau so groß,
                // wie ADL mitgeteilt wird, und `PmLogData` entspricht `ADLPMLogDataOutput`.
                unsafe {
                    let mut adapters = 0i32;
                    if (self.count)(self.context, &mut adapters) != 0 || adapters <= 0 {
                        return None;
                    }
                    let adapters = (adapters as usize).min(MAX_ADAPTERS);
                    let mut buffer = vec![0u8; adapters * ADAPTER_INFO_SIZE];
                    if (self.info)(self.context, buffer.as_mut_ptr(), buffer.len() as i32) != 0 {
                        return None;
                    }
                    let field = |entry: &[u8], offset: usize| {
                        i32::from_ne_bytes([
                            entry[offset],
                            entry[offset + 1],
                            entry[offset + 2],
                            entry[offset + 3],
                        ])
                    };

                    // ADL führt je Bildschirmausgang einen Eintrag – jede Karte (PCI-Bus)
                    // nur einmal zählen.
                    let mut seen_buses = HashSet::new();
                    let mut total: Option<f32> = None;
                    for entry in buffer.chunks_exact(ADAPTER_INFO_SIZE) {
                        if !seen_buses.insert(field(entry, OFFSET_BUS)) {
                            continue;
                        }
                        let index = field(entry, OFFSET_INDEX);
                        // Die Leistung einer im Prozessor integrierten Grafik steckt schon
                        // in der gemessenen Prozessorleistung – nicht doppelt zählen.
                        if let Some(family) = self.family {
                            let (mut types, mut valid) = (0i32, 0i32);
                            if family(self.context, index, &mut types, &mut valid) == 0
                                && types & valid & ASIC_IN_PROCESSOR != 0
                            {
                                continue;
                            }
                        }
                        let mut data: PmLogData = mem::zeroed();
                        if (self.query)(self.context, index, &mut data) != 0 {
                            continue;
                        }
                        let sensor = data.sensors[SENSOR_ASIC_POWER];
                        if sensor.supported != 0 && (1..2000).contains(&sensor.value) {
                            total = Some(total.unwrap_or(0.0) + sensor.value as f32);
                        }
                    }
                    total
                }
            }
        }

        impl Drop for Adl {
            fn drop(&mut self) {
                // SAFETY: `context` ist der in `load` erzeugte, noch gültige Kontext.
                unsafe {
                    (self.destroy)(self.context);
                }
            }
        }
    }

    /// Abfrage über den Windows-Grafikkern (D3DKMT) – herstellerunabhängig.
    mod kernel {
        use std::{collections::HashMap, ffi::c_void, mem, ptr};

        use super::Adapter;

        /// Abfrageart „Namen aus der Registrierung“ (`KMTQAITYPE_ADAPTERREGISTRYINFO`).
        const ADAPTER_REGISTRY_INFO: u32 = 8;
        /// Abfrageart „Art des Adapters“ (`KMTQAITYPE_ADAPTERTYPE`), ein Bitfeld.
        const ADAPTER_TYPE: u32 = 15;
        const TYPE_RENDER: u32 = 1 << 0;
        const TYPE_SOFTWARE: u32 = 1 << 2;
        /// Virtuelle Bildschirmtreiber (Fernzugriff, virtuelle Monitore).
        const TYPE_INDIRECT_DISPLAY: u32 = 1 << 6;
        /// Abfrageart „Leistungsdaten des Adapters“ (`KMTQAITYPE_ADAPTERPERFDATA`).
        const ADAPTER_PERF_DATA: u32 = 62;
        const MAX_ADAPTERS: usize = 16;
        const NAME_LEN: usize = 260;

        #[repr(C)]
        #[derive(Clone, Copy)]
        struct AdapterInfo {
            handle: u32,
            luid_low: u32,
            luid_high: i32,
            sources: u32,
            precise_present_regions: i32,
        }

        #[repr(C)]
        struct EnumAdapters {
            count: u32,
            adapters: *mut AdapterInfo,
        }

        #[repr(C)]
        struct QueryAdapterInfo {
            handle: u32,
            kind: u32,
            data: *mut c_void,
            size: u32,
        }

        #[repr(C)]
        struct RegistryInfo {
            adapter: [u16; NAME_LEN],
            bios: [u16; NAME_LEN],
            dac: [u16; NAME_LEN],
            chip: [u16; NAME_LEN],
        }

        #[repr(C)]
        struct PerfData {
            physical_adapter_index: u32,
            memory_frequency: u64,
            max_memory_frequency: u64,
            max_memory_frequency_oc: u64,
            memory_bandwidth: u64,
            pcie_bandwidth: u64,
            fan_rpm: u32,
            power: u32,
            /// Zehntel Grad Celsius.
            temperature: u32,
            power_state_override: u8,
        }

        #[repr(C)]
        struct CloseAdapter {
            handle: u32,
        }

        #[link(name = "gdi32")]
        extern "system" {
            fn D3DKMTEnumAdapters2(request: *mut EnumAdapters) -> i32;
            fn D3DKMTQueryAdapterInfo(request: *mut QueryAdapterInfo) -> i32;
            fn D3DKMTCloseAdapter(request: *const CloseAdapter) -> i32;
        }

        /// Alle echten Grafikkarten mit Name und Temperatur. `names` merkt sich die Namen.
        pub fn adapters(names: &mut HashMap<u64, String>) -> Vec<Adapter> {
            let mut found = Vec::new();
            // SAFETY: Alle Strukturen entsprechen den Windows-Definitionen und leben während
            // der Aufrufe. Der Adapterpuffer fasst `MAX_ADAPTERS` Einträge, und genau so viele
            // werden Windows als Obergrenze genannt. Jedes gelieferte Handle wird geschlossen.
            unsafe {
                let mut list: [AdapterInfo; MAX_ADAPTERS] = mem::zeroed();
                let mut request = EnumAdapters {
                    count: MAX_ADAPTERS as u32,
                    adapters: list.as_mut_ptr(),
                };
                if D3DKMTEnumAdapters2(&mut request) != 0 {
                    return found;
                }

                for adapter in list.iter().take((request.count as usize).min(MAX_ADAPTERS)) {
                    let luid = ((adapter.luid_high as u32 as u64) << 32) | adapter.luid_low as u64;

                    if !names.contains_key(&luid) {
                        let mut info: RegistryInfo = mem::zeroed();
                        let mut query = QueryAdapterInfo {
                            handle: adapter.handle,
                            kind: ADAPTER_REGISTRY_INFO,
                            data: ptr::addr_of_mut!(info).cast(),
                            size: mem::size_of::<RegistryInfo>() as u32,
                        };
                        let name = if D3DKMTQueryAdapterInfo(&mut query) == 0 {
                            let len = info
                                .adapter
                                .iter()
                                .position(|&c| c == 0)
                                .unwrap_or(NAME_LEN);
                            String::from_utf16_lossy(&info.adapter[..len])
                                .trim()
                                .to_owned()
                        } else {
                            String::new()
                        };
                        names.insert(luid, name);
                    }
                    let name = names.get(&luid).cloned().unwrap_or_default();

                    let mut data: PerfData = mem::zeroed();
                    let mut query = QueryAdapterInfo {
                        handle: adapter.handle,
                        kind: ADAPTER_PERF_DATA,
                        data: ptr::addr_of_mut!(data).cast(),
                        size: mem::size_of::<PerfData>() as u32,
                    };
                    let temperature = (D3DKMTQueryAdapterInfo(&mut query) == 0)
                        .then(|| data.temperature as f32 / 10.0)
                        .filter(|celsius| *celsius > 0.0 && *celsius < 150.0);
                    let mut kind = 0u32;
                    let mut query = QueryAdapterInfo {
                        handle: adapter.handle,
                        kind: ADAPTER_TYPE,
                        data: ptr::addr_of_mut!(kind).cast(),
                        size: mem::size_of::<u32>() as u32,
                    };
                    if D3DKMTQueryAdapterInfo(&mut query) != 0 {
                        kind = 0; // unbekannt: nicht aussortieren
                    }
                    D3DKMTCloseAdapter(&CloseAdapter {
                        handle: adapter.handle,
                    });

                    // Der Software-Renderer von Windows ist keine Grafikkarte.
                    if name.contains("Basic Render")
                        || kind & (TYPE_SOFTWARE | TYPE_INDIRECT_DISPLAY) != 0
                        || (kind != 0 && kind & TYPE_RENDER == 0)
                    {
                        continue;
                    }
                    found.push(Adapter {
                        luids: vec![luid],
                        name,
                        temperature,
                    });
                }
            }
            found
        }
    }
}
