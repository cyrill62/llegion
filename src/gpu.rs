//! Cross-platform GPU detection and live sampling.
//!
//! - NVIDIA: `nvidia-smi` (Linux, Windows, WSL)
//! - AMD / Intel on Linux: `/sys/class/drm` (+ `lspci` for marketing names)
//! - Windows (non-NVIDIA): display adapter registry keys via PowerShell; live usage and
//!   dedicated VRAM from the GPU performance counters (the ones Task Manager shows)
//! - macOS: `system_profiler SPDisplaysDataType`

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::util::run_cmd;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Apple,
    Other,
}

impl Vendor {
    pub fn label(self) -> &'static str {
        match self {
            Vendor::Nvidia => "NVIDIA",
            Vendor::Amd => "AMD",
            Vendor::Intel => "Intel",
            Vendor::Apple => "Apple",
            Vendor::Other => "Other",
        }
    }

    #[cfg_attr(target_os = "macos", allow(dead_code))]
    fn from_pci_id(id: u32) -> Self {
        match id {
            0x10de => Vendor::Nvidia,
            0x1002 => Vendor::Amd,
            0x8086 => Vendor::Intel,
            0x106b => Vendor::Apple,
            _ => Vendor::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuKind {
    Discrete,
    Integrated,
    /// Apple Silicon: GPU shares system memory, not something to "disable".
    Unified,
}

impl GpuKind {
    pub fn label(self) -> &'static str {
        match self {
            GpuKind::Discrete => "Discrete",
            GpuKind::Integrated => "Integrated",
            GpuKind::Unified => "Unified memory",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub vendor: Vendor,
    pub name: String,
    pub kind: GpuKind,
    pub vram_total: u64,
    pub vram_used: Option<u64>,
    pub utilization: Option<f32>,
    pub temperature: Option<f32>,
    pub driver: Option<String>,
    /// Identifier understood by the backend's `*_VISIBLE_DEVICES` variable.
    pub backend_id: String,
    /// Where live metrics come from.
    source: Source,
}

#[derive(Debug, Clone)]
enum Source {
    NvidiaSmi {
        uuid: String,
    },
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Sysfs {
        card_dir: String,
    },
    /// Windows performance counters; `luid` as written in their instance names.
    #[cfg_attr(not(windows), allow(dead_code))]
    WindowsPdh {
        luid: String,
    },
    Static,
}

impl GpuInfo {
    /// Stable key for persisting per-device preferences.
    pub fn key(&self) -> String {
        format!("{}:{}:{}", self.vendor.label(), self.backend_id, self.name)
    }
}

pub fn detect() -> Vec<GpuInfo> {
    let mut gpus = detect_nvidia();
    #[cfg(target_os = "linux")]
    gpus.extend(detect_linux_sysfs());
    #[cfg(windows)]
    gpus.extend(detect_windows(&gpus));
    #[cfg(target_os = "macos")]
    gpus.extend(detect_macos());
    gpus
}

/// Refreshes VRAM usage, utilisation and temperature in place.
pub fn sample(gpus: &mut [GpuInfo]) {
    if gpus
        .iter()
        .any(|g| matches!(g.source, Source::NvidiaSmi { .. }))
    {
        let rows = query_nvidia();
        for gpu in gpus.iter_mut() {
            if let Source::NvidiaSmi { uuid } = &gpu.source
                && let Some(row) = rows.iter().find(|r| &r.uuid == uuid)
            {
                gpu.vram_used = Some(row.mem_used);
                gpu.utilization = row.util;
                gpu.temperature = row.temp;
            }
        }
    }
    for gpu in gpus.iter_mut() {
        if let Source::Sysfs { card_dir } = &gpu.source {
            let dev = format!("{card_dir}/device");
            gpu.vram_used = read_u64(&format!("{dev}/mem_info_vram_used"));
            gpu.utilization = read_u64(&format!("{dev}/gpu_busy_percent")).map(|v| v as f32);
            gpu.temperature = hwmon_temp(&dev);
        }
    }
    #[cfg(windows)]
    pdh::sample(gpus);
}

// ---------------------------------------------------------------------------
// NVIDIA

struct NvRow {
    index: String,
    uuid: String,
    name: String,
    mem_total: u64,
    mem_used: u64,
    util: Option<f32>,
    temp: Option<f32>,
    driver: String,
}

fn nvidia_smi_path() -> &'static str {
    // WSL ships the binary outside of $PATH.
    if cfg!(target_os = "linux") && std::path::Path::new("/usr/lib/wsl/lib/nvidia-smi").exists() {
        "/usr/lib/wsl/lib/nvidia-smi"
    } else {
        "nvidia-smi"
    }
}

fn query_nvidia() -> Vec<NvRow> {
    run_cmd(
        nvidia_smi_path(),
        &[
            "--query-gpu=index,uuid,name,memory.total,memory.used,utilization.gpu,temperature.gpu,driver_version",
            "--format=csv,noheader,nounits",
        ],
    )
    .map(|out| parse_nvidia_csv(&out))
    .unwrap_or_default()
}

/// Parses `nvidia-smi --format=csv,noheader,nounits` output. Unsupported fields read `[N/A]`.
fn parse_nvidia_csv(out: &str) -> Vec<NvRow> {
    const MIB: u64 = 1024 * 1024;
    out.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(',').map(str::trim).collect();
            if f.len() < 8 {
                return None;
            }
            Some(NvRow {
                index: f[0].to_string(),
                uuid: f[1].to_string(),
                name: f[2].to_string(),
                mem_total: f[3].parse::<u64>().ok()? * MIB,
                mem_used: f[4].parse::<u64>().unwrap_or(0) * MIB,
                util: f[5].parse().ok(),
                temp: f[6].parse().ok(),
                driver: f[7].to_string(),
            })
        })
        .collect()
}

fn detect_nvidia() -> Vec<GpuInfo> {
    query_nvidia()
        .into_iter()
        .map(|r| GpuInfo {
            vendor: Vendor::Nvidia,
            // Jetson/Grace and some laptop parts report shared memory; treat all as discrete.
            kind: GpuKind::Discrete,
            name: r.name,
            vram_total: r.mem_total,
            vram_used: Some(r.mem_used),
            utilization: r.util,
            temperature: r.temp,
            driver: Some(r.driver),
            backend_id: if r.uuid.starts_with("GPU-") {
                r.uuid.clone()
            } else {
                r.index
            },
            source: Source::NvidiaSmi { uuid: r.uuid },
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Heuristics shared by Linux and Windows

/// Intel Arc discrete device IDs (Alchemist 0x56xx, Battlemage 0xe20x/0xe21x).
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn intel_is_discrete(device_id: u32) -> bool {
    (0x5690..=0x56ff).contains(&device_id) || (0xe200..=0xe2ff).contains(&device_id)
}

/// AMD APUs expose a small carve-out (typically 512 MB – 2 GB) as "VRAM".
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn amd_is_integrated(vram_total: u64, name: &str) -> bool {
    let n = name.to_lowercase();
    if n.contains("rx ") || n.contains("radeon pro") || n.contains("instinct") {
        return false;
    }
    vram_total <= 2 * 1024 * 1024 * 1024
        || n.ends_with("radeon graphics")
        || n.contains("radeon(tm) graphics")
        || n.contains("vega 8")
        || n.contains("780m")
        || n.contains("890m")
}

// ---------------------------------------------------------------------------
// Linux

#[cfg(target_os = "linux")]
fn read_u64(path: &str) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(not(target_os = "linux"))]
fn read_u64(_path: &str) -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn read_hex(path: &str) -> Option<u32> {
    let s = std::fs::read_to_string(path).ok()?;
    u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
}

#[cfg(target_os = "linux")]
fn hwmon_temp(device_dir: &str) -> Option<f32> {
    let entries = std::fs::read_dir(format!("{device_dir}/hwmon")).ok()?;
    for entry in entries.flatten() {
        if let Some(milli) = read_u64(&format!("{}/temp1_input", entry.path().display())) {
            return Some(milli as f32 / 1000.0);
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn hwmon_temp(_device_dir: &str) -> Option<f32> {
    None
}

#[cfg(target_os = "linux")]
fn lspci_name(slot: &str) -> Option<String> {
    // -mm: machine readable: "slot" "class" "vendor" "device" ...
    let out = run_cmd("lspci", &["-mm", "-s", slot])?;
    let fields: Vec<&str> = out.split('"').filter(|s| !s.trim().is_empty()).collect();
    let vendor = fields.get(2)?.trim();
    let device = fields.get(3)?.trim();
    Some(format!("{vendor} {device}"))
}

#[cfg(target_os = "linux")]
fn detect_linux_sysfs() -> Vec<GpuInfo> {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut cards: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("card") && n[4..].chars().all(|c| c.is_ascii_digit()))
        .collect();
    cards.sort_by_key(|n| n[4..].parse::<u32>().unwrap_or(0));

    let mut gpus = Vec::new();
    let mut amd_index = 0;
    for card in cards {
        let card_dir = format!("/sys/class/drm/{card}");
        let dev = format!("{card_dir}/device");
        let Some(vendor_id) = read_hex(&format!("{dev}/vendor")) else {
            continue;
        };
        let vendor = Vendor::from_pci_id(vendor_id);
        // NVIDIA is handled by nvidia-smi which has far better data.
        if !matches!(vendor, Vendor::Amd | Vendor::Intel) {
            continue;
        }
        let device_id = read_hex(&format!("{dev}/device")).unwrap_or(0);
        let slot = std::fs::read_to_string(format!("{dev}/uevent"))
            .ok()
            .and_then(|u| {
                u.lines()
                    .find_map(|l| l.strip_prefix("PCI_SLOT_NAME=").map(str::to_string))
            });
        let name = slot
            .as_deref()
            .and_then(lspci_name)
            .unwrap_or_else(|| format!("{} GPU [{device_id:04x}]", vendor.label()));
        let vram_total = read_u64(&format!("{dev}/mem_info_vram_total")).unwrap_or(0);
        let kind = match vendor {
            Vendor::Amd if amd_is_integrated(vram_total, &name) => GpuKind::Integrated,
            Vendor::Intel if !intel_is_discrete(device_id) => GpuKind::Integrated,
            _ => GpuKind::Discrete,
        };
        let backend_id = if vendor == Vendor::Amd {
            let id = amd_index.to_string();
            amd_index += 1;
            id
        } else {
            "0".into()
        };
        let driver = std::fs::read_link(format!("{dev}/driver"))
            .ok()
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()));
        let mut gpu = GpuInfo {
            vendor,
            name,
            kind,
            vram_total,
            vram_used: None,
            utilization: None,
            temperature: None,
            driver,
            backend_id,
            source: Source::Sysfs { card_dir },
        };
        sample(std::slice::from_mut(&mut gpu));
        gpus.push(gpu);
    }
    gpus
}

// ---------------------------------------------------------------------------
// Windows

/// Lists adapters that are actually present (`Win32_VideoController`) and joins the
/// display-class registry key for the real 64-bit VRAM size (`AdapterRAM` caps at 4 GB).
#[cfg(windows)]
const WINDOWS_GPU_SCRIPT: &str = r#"
$regs = Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\0*' -ErrorAction SilentlyContinue
@(Get-CimInstance Win32_VideoController | ForEach-Object {
    $vc = $_
    $reg = $regs | Where-Object { $_.DriverDesc -eq $vc.Name } | Select-Object -First 1
    [pscustomobject]@{
        Name = $vc.Name
        PnpId = $vc.PNPDeviceID
        DriverVersion = $vc.DriverVersion
        AdapterRAM = $vc.AdapterRAM
        Mem = $reg.'HardwareInformation.qwMemorySize'
    }
}) | ConvertTo-Json -Compress
"#;

#[cfg(windows)]
fn detect_windows(already: &[GpuInfo]) -> Vec<GpuInfo> {
    run_cmd(
        "powershell",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            WINDOWS_GPU_SCRIPT,
        ],
    )
    .map(|out| {
        let mut gpus = parse_windows_adapters(&out, !already.is_empty());
        pdh::attach_luids(&mut gpus);
        gpus
    })
    .unwrap_or_default()
}

/// `luid_0x00000000_0x0000c2e5` from `pid_42_luid_0x00000000_0x0000C2E5_phys_0_eng_3_engtype_3D`.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn pdh_luid(instance: &str) -> Option<String> {
    let at = instance.to_ascii_lowercase().find("luid_")?;
    instance
        .get(at..at + 26)
        .map(|luid| luid.to_ascii_lowercase())
}

/// Usage per adapter like Task Manager: each engine sums its processes, the adapter
/// shows its busiest engine.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn engine_utilization(items: &[(String, f64)]) -> HashMap<String, f32> {
    let mut engines: HashMap<String, f64> = HashMap::new();
    for (instance, value) in items {
        let lower = instance.to_ascii_lowercase();
        if let Some(at) = lower.find("luid_") {
            *engines.entry(lower[at..].to_string()).or_default() += value;
        }
    }
    let mut adapters: HashMap<String, f32> = HashMap::new();
    for (engine, value) in engines {
        if let Some(luid) = pdh_luid(&engine) {
            let busiest = adapters.entry(luid).or_default();
            *busiest = busiest.max(value.min(100.0) as f32);
        }
    }
    adapters
}

/// Dedicated memory in use per adapter, summed over its physical parts.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn adapter_memory(items: &[(String, f64)]) -> HashMap<String, u64> {
    let mut adapters: HashMap<String, u64> = HashMap::new();
    for (instance, value) in items {
        if let Some(luid) = pdh_luid(instance) {
            *adapters.entry(luid).or_default() += *value as u64;
        }
    }
    adapters
}

#[cfg(windows)]
mod pdh {
    use std::sync::Mutex;

    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1,
    };
    use windows::Win32::System::Performance::{
        PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
        PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
    };
    use windows::core::w;

    use super::{GpuInfo, Source, adapter_memory, engine_utilization};

    /// Links the adapters found by PowerShell to their LUID, which names the counters.
    pub fn attach_luids(gpus: &mut [GpuInfo]) {
        let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
            return;
        };
        let mut i = 0;
        while let Ok(adapter) = unsafe { factory.EnumAdapters1(i) } {
            i += 1;
            let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
                continue;
            };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let len = desc.Description.iter().position(|&c| c == 0).unwrap_or(128);
            let name = String::from_utf16_lossy(&desc.Description[..len]);
            // Identical cards are matched in enumeration order.
            if let Some(gpu) = gpus
                .iter_mut()
                .find(|g| matches!(g.source, Source::Static) && g.name == name.trim())
            {
                gpu.source = Source::WindowsPdh {
                    luid: format!(
                        "luid_0x{:08x}_0x{:08x}",
                        desc.AdapterLuid.HighPart as u32, desc.AdapterLuid.LowPart
                    ),
                };
            }
        }
    }

    /// Kept open between samples: usage is a rate between two collections.
    struct Query {
        query: PDH_HQUERY,
        engine: PDH_HCOUNTER,
        memory: PDH_HCOUNTER,
    }

    // The handles are only used under the mutex.
    unsafe impl Send for Query {}

    static QUERY: Mutex<Option<Query>> = Mutex::new(None);

    fn open() -> Option<Query> {
        let mut q = Query {
            query: PDH_HQUERY::default(),
            engine: PDH_HCOUNTER::default(),
            memory: PDH_HCOUNTER::default(),
        };
        unsafe {
            (PdhOpenQueryW(None, 0, &mut q.query) == 0).then_some(())?;
            (PdhAddEnglishCounterW(
                q.query,
                w!("\\GPU Engine(*)\\Utilization Percentage"),
                0,
                &mut q.engine,
            ) == 0)
                .then_some(())?;
            (PdhAddEnglishCounterW(
                q.query,
                w!("\\GPU Adapter Memory(*)\\Dedicated Usage"),
                0,
                &mut q.memory,
            ) == 0)
                .then_some(())?;
            PdhCollectQueryData(q.query);
        }
        Some(q)
    }

    /// Current value of every instance of a wildcard counter.
    fn read(counter: PDH_HCOUNTER) -> Option<Vec<(String, f64)>> {
        let (mut size, mut count) = (0u32, 0u32);
        unsafe {
            if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None)
                != PDH_MORE_DATA
            {
                return None;
            }
            // u64 storage keeps the items aligned.
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            let items = buf.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            if PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                Some(items),
            ) != 0
            {
                return None;
            }
            Some(
                std::slice::from_raw_parts(items, count as usize)
                    .iter()
                    // PDH_CSTATUS_VALID_DATA / PDH_CSTATUS_NEW_DATA
                    .filter(|item| item.FmtValue.CStatus <= 1)
                    .filter_map(|item| {
                        Some((
                            item.szName.to_string().ok()?,
                            item.FmtValue.Anonymous.doubleValue,
                        ))
                    })
                    .collect(),
            )
        }
    }

    pub fn sample(gpus: &mut [GpuInfo]) {
        if !gpus
            .iter()
            .any(|g| matches!(g.source, Source::WindowsPdh { .. }))
        {
            return;
        }
        let mut query = QUERY.lock().unwrap();
        if query.is_none() {
            *query = open();
        }
        let Some(q) = query.as_ref() else {
            return;
        };
        if unsafe { PdhCollectQueryData(q.query) } != 0 {
            return;
        }
        let usage = read(q.engine).map(|items| engine_utilization(&items));
        let memory = read(q.memory).map(|items| adapter_memory(&items));
        for gpu in gpus.iter_mut() {
            if let Source::WindowsPdh { luid } = &gpu.source {
                // An adapter no process is using has no engine instance: it is idle.
                gpu.utilization = usage.as_ref().map(|u| u.get(luid).copied().unwrap_or(0.0));
                gpu.vram_used = memory.as_ref().and_then(|m| m.get(luid).copied());
            }
        }
    }
}

/// Parses the JSON produced by `WINDOWS_GPU_SCRIPT`. NVIDIA entries are skipped when
/// `nvidia-smi` already reported them.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn parse_windows_adapters(json: &str, skip_nvidia: bool) -> Vec<GpuInfo> {
    let items = match serde_json::from_str::<serde_json::Value>(json.trim()) {
        Ok(serde_json::Value::Array(a)) => a,
        Ok(v @ serde_json::Value::Object(_)) => vec![v],
        _ => return Vec::new(),
    };
    let mut gpus = Vec::new();
    let mut amd_index = 0;
    for item in items {
        let name = item["Name"].as_str().unwrap_or_default().trim().to_string();
        if name.is_empty()
            || name.contains("Basic Display")
            || name.contains("Basic Render")
            || name.contains("Remote")
            || name.contains("Virtual")
        {
            continue;
        }
        let ids = item["PnpId"].as_str().unwrap_or_default().to_lowercase();
        let hex_after = |tag: &str| {
            let s = ids.split(tag).nth(1)?;
            u32::from_str_radix(s.get(..4)?, 16).ok()
        };
        let vendor = hex_after("ven_")
            .map(Vendor::from_pci_id)
            .unwrap_or(Vendor::Other);
        if vendor == Vendor::Nvidia && skip_nvidia {
            continue;
        }
        let device_id = hex_after("dev_").unwrap_or(0);
        let vram_total = item["Mem"]
            .as_u64()
            .or_else(|| item["AdapterRAM"].as_u64())
            .unwrap_or(0);
        let kind = match vendor {
            Vendor::Amd if amd_is_integrated(vram_total, &name) => GpuKind::Integrated,
            Vendor::Intel if !intel_is_discrete(device_id) => GpuKind::Integrated,
            _ => GpuKind::Discrete,
        };
        let backend_id = if vendor == Vendor::Amd {
            let id = amd_index.to_string();
            amd_index += 1;
            id
        } else {
            "0".into()
        };
        gpus.push(GpuInfo {
            vendor,
            name,
            kind,
            vram_total,
            vram_used: None,
            utilization: None,
            temperature: None,
            driver: item["DriverVersion"].as_str().map(str::to_string),
            backend_id,
            source: Source::Static,
        });
    }
    gpus
}

// ---------------------------------------------------------------------------
// macOS

#[cfg(target_os = "macos")]
fn detect_macos() -> Vec<GpuInfo> {
    let Some(out) = run_cmd("system_profiler", &["SPDisplaysDataType", "-json"]) else {
        return Vec::new();
    };
    let total_ram = {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        sys.total_memory()
    };
    parse_macos_displays(&out, total_ram)
}

/// Parses `system_profiler SPDisplaysDataType -json`. Apple Silicon GPUs share system RAM.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn parse_macos_displays(json: &str, total_ram: u64) -> Vec<GpuInfo> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let parse_vram = |s: &str| -> Option<u64> {
        let mut parts = s.split_whitespace();
        let n: u64 = parts.next()?.parse().ok()?;
        Some(match parts.next() {
            Some("GB") => n * 1024 * 1024 * 1024,
            _ => n * 1024 * 1024,
        })
    };
    let mut gpus = Vec::new();
    for (i, item) in value["SPDisplaysDataType"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let name = item["sppci_model"].as_str().unwrap_or("GPU").to_string();
        // e.g. "sppci_vendor_Apple", "sppci_vendor_amd", "sppci_vendor_intel"
        let vendor_str = item["spdisplays_vendor"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase();
        let vendor = if vendor_str.contains("apple") || name.starts_with("Apple") {
            Vendor::Apple
        } else if vendor_str.contains("amd") || vendor_str.contains("_ati") {
            Vendor::Amd
        } else if vendor_str.contains("intel") {
            Vendor::Intel
        } else if vendor_str.contains("nvidia") {
            Vendor::Nvidia
        } else {
            Vendor::Other
        };
        let (kind, vram_total) = if vendor == Vendor::Apple {
            (GpuKind::Unified, total_ram)
        } else if let Some(v) = item["spdisplays_vram"].as_str().and_then(parse_vram) {
            (GpuKind::Discrete, v)
        } else {
            let shared = item["spdisplays_vram_shared"]
                .as_str()
                .and_then(parse_vram)
                .unwrap_or(0);
            (GpuKind::Integrated, shared)
        };
        gpus.push(GpuInfo {
            vendor,
            name,
            kind,
            vram_total,
            vram_used: None,
            utilization: None,
            temperature: None,
            driver: item["spdisplays_mtlgpufamilysupport"]
                .as_str()
                .map(str::to_string),
            backend_id: i.to_string(),
            source: Source::Static,
        });
    }
    gpus
}

#[cfg(test)]
pub fn test_gpu(vendor: Vendor, kind: GpuKind, backend_id: &str) -> GpuInfo {
    GpuInfo {
        vendor,
        name: format!("{} test", vendor.label()),
        kind,
        vram_total: 8 << 30,
        vram_used: None,
        utilization: None,
        temperature: None,
        driver: None,
        backend_id: backend_id.into(),
        source: Source::Static,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    /// Runs the real platform probes (PowerShell, system_profiler, sysfs, nvidia-smi).
    /// CI runners usually have no real GPU, so this only checks that nothing panics.
    #[test]
    fn detect_and_sample_do_not_panic() {
        let mut gpus = detect();
        sample(&mut gpus);
        for g in &gpus {
            println!(
                "{} {} {:?} {} bytes",
                g.vendor.label(),
                g.name,
                g.kind,
                g.vram_total
            );
        }
    }

    #[test]
    fn nvidia_csv() {
        let out = "0, GPU-1a2b, NVIDIA GeForce RTX 4090, 24564, 1234, 7, 41, 581.15\n\
                   1, GPU-3c4d, NVIDIA RTX A2000, 6138, 0, [N/A], [N/A], 581.15\n\
                   garbage line\n";
        let rows = parse_nvidia_csv(out);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "NVIDIA GeForce RTX 4090");
        assert_eq!(rows[0].mem_total, 24564 * 1024 * 1024);
        assert_eq!(rows[0].util, Some(7.0));
        assert_eq!(rows[1].util, None);
        assert_eq!(rows[1].uuid, "GPU-3c4d");
    }

    #[test]
    fn windows_counters() {
        let engines = [
            (
                "pid_10_luid_0x00000000_0x0000C2E5_phys_0_eng_0_engtype_3D".to_string(),
                30.0,
            ),
            (
                "pid_11_luid_0x00000000_0x0000C2E5_phys_0_eng_0_engtype_3D".into(),
                25.0,
            ),
            (
                "pid_11_luid_0x00000000_0x0000C2E5_phys_0_eng_3_engtype_Compute".into(),
                70.0,
            ),
            (
                "pid_12_luid_0x00000000_0x0000D1B5_phys_0_eng_0_engtype_3D".into(),
                80.0,
            ),
            (
                "pid_13_luid_0x00000000_0x0000D1B5_phys_0_eng_0_engtype_3D".into(),
                60.0,
            ),
        ];
        let usage = engine_utilization(&engines);
        // Busiest engine, its processes summed, capped at 100.
        assert_eq!(usage["luid_0x00000000_0x0000c2e5"], 70.0);
        assert_eq!(usage["luid_0x00000000_0x0000d1b5"], 100.0);

        let memory = adapter_memory(&[
            ("luid_0x00000000_0x0000C2E5_phys_0".to_string(), 1024.0),
            ("luid_0x00000000_0x0000C2E5_phys_1".into(), 512.0),
        ]);
        assert_eq!(memory["luid_0x00000000_0x0000c2e5"], 1536);
        assert_eq!(pdh_luid("no luid here"), None);
    }

    #[test]
    fn windows_adapters() {
        let json = r#"[
            {"Name":"NVIDIA GeForce RTX 4070","PnpId":"PCI\\VEN_10DE&DEV_2786&SUBSYS_00000000","DriverVersion":"32.0.15.8115","AdapterRAM":4293918720,"Mem":12878610432},
            {"Name":"AMD Radeon(TM) Graphics","PnpId":"PCI\\VEN_1002&DEV_164E&SUBSYS_88771043","DriverVersion":"31.0.21921.1000","AdapterRAM":536870912,"Mem":null},
            {"Name":"Intel(R) Arc(TM) B580 Graphics","PnpId":"PCI\\VEN_8086&DEV_E20B","DriverVersion":"32.0.101.6297","AdapterRAM":4293918720,"Mem":12884901888},
            {"Name":"Intel(R) UHD Graphics 770","PnpId":"PCI\\VEN_8086&DEV_4680","DriverVersion":"31.0.101.4502","AdapterRAM":1073741824,"Mem":null},
            {"Name":"Microsoft Basic Render Driver","PnpId":"SWD\\REMOTEDISPLAYENUM","DriverVersion":"10.0","AdapterRAM":0,"Mem":null}
        ]"#;
        let gpus = parse_windows_adapters(json, false);
        assert_eq!(gpus.len(), 4);
        assert_eq!(gpus[0].vendor, Vendor::Nvidia);
        assert_eq!(
            gpus[0].vram_total, 12878610432,
            "registry size wins over capped AdapterRAM"
        );
        assert_eq!(gpus[1].kind, GpuKind::Integrated);
        assert_eq!(gpus[1].backend_id, "0");
        assert_eq!(gpus[2].kind, GpuKind::Discrete, "Arc is discrete");
        assert_eq!(gpus[3].kind, GpuKind::Integrated);
        // Skip NVIDIA when nvidia-smi already listed it; a single object (not array) also parses.
        assert_eq!(parse_windows_adapters(json, true).len(), 3);
        let single = r#"{"Name":"AMD Radeon RX 7900 XTX","PnpId":"PCI\\VEN_1002&DEV_744C","Mem":25753026560}"#;
        let g = parse_windows_adapters(single, false);
        assert_eq!((g[0].vendor, g[0].kind), (Vendor::Amd, GpuKind::Discrete));
    }

    #[test]
    fn macos_displays() {
        let json = r#"{"SPDisplaysDataType":[
            {"sppci_model":"Apple M3 Max","spdisplays_vendor":"sppci_vendor_Apple","spdisplays_mtlgpufamilysupport":"spdisplays_metal3"},
            {"sppci_model":"AMD Radeon Pro 5500M","spdisplays_vendor":"sppci_vendor_amd","spdisplays_vram":"8 GB"},
            {"sppci_model":"Intel UHD Graphics 630","spdisplays_vendor":"sppci_vendor_intel","spdisplays_vram_shared":"1536 MB"}
        ]}"#;
        let gpus = parse_macos_displays(json, 64 * GIB);
        assert_eq!(gpus.len(), 3);
        assert_eq!(
            (gpus[0].vendor, gpus[0].kind, gpus[0].vram_total),
            (Vendor::Apple, GpuKind::Unified, 64 * GIB)
        );
        assert_eq!(
            (gpus[1].vendor, gpus[1].kind, gpus[1].vram_total),
            (Vendor::Amd, GpuKind::Discrete, 8 * GIB)
        );
        assert_eq!(
            (gpus[2].vendor, gpus[2].kind),
            (Vendor::Intel, GpuKind::Integrated)
        );
        assert_eq!(gpus[2].vram_total, 1536 * 1024 * 1024);
    }
}
