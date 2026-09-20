//! Detección del PC del empleado.
//!
//! Todo lo que la recomendación de modelos necesita saber de la máquina, en una
//! sola estructura serializable. La detección nunca falla: cada pieza que no se
//! puede obtener queda a `None` o a un valor neutro y el resto sigue.

pub mod gpu;

pub use gpu::{GpuInfo, GpuKind};

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

/// Acelerador de inferencia efectivo de esta máquina.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Accelerator {
    Metal,
    Cuda,
    Vulkan,
    OpenCl,
    Cpu,
}

impl Accelerator {
    pub fn label(self) -> &'static str {
        match self {
            Self::Metal => "Metal",
            Self::Cuda => "CUDA",
            Self::Vulkan => "Vulkan",
            Self::OpenCl => "OpenCL",
            Self::Cpu => "CPU",
        }
    }

    /// Cuántas copias de los pesos hay que presupuestar.
    ///
    /// CUDA y Vulkan sobre GPU discreta copian los pesos del `mmap` a memoria del
    /// dispositivo, así que durante la carga conviven dos copias. Metal (memoria
    /// unificada), CPU y OpenCL mantienen el `mmap`. Criterio heredado de Rebost.
    pub fn weight_copies(self) -> f64 {
        match self {
            Self::Cuda | Self::Vulkan => 2.0,
            Self::Metal | Self::Cpu | Self::OpenCl => 1.15,
        }
    }

    /// Ancho de banda de memoria típico en GB/s, usado **solo** para la
    /// estimación orientativa de tokens/s que se muestra antes de ejecutar.
    pub fn typical_bandwidth_gbs(self) -> f64 {
        match self {
            Self::Cuda => 600.0,
            Self::Metal => 200.0,
            Self::Vulkan => 300.0,
            Self::OpenCl => 60.0,
            Self::Cpu => 40.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProfile {
    pub os: String,
    pub os_version: String,
    pub hostname: String,
    pub arch: String,
    pub cpu_brand: String,
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub total_ram_bytes: u64,
    pub available_ram_bytes: u64,
    pub free_disk_bytes: u64,
    pub total_disk_bytes: u64,
    pub gpus: Vec<GpuInfo>,
    pub accelerator: Accelerator,
    /// `true` en Apple Silicon: VRAM y RAM son la misma memoria.
    pub unified_memory: bool,
    /// VRAM utilizable para alojar pesos, ya descontadas las GPU integradas.
    pub usable_vram_bytes: u64,
}

impl HardwareProfile {
    /// RAM que un modelo puede usar sin ahogar el equipo.
    ///
    /// El 0,65 deja sitio al sistema, al navegador y a la ofimática, que es lo
    /// que un empleado tiene abierto a la vez. Mismo factor que Rebost.
    pub const RAM_BUDGET_FACTOR: f64 = 0.65;
    /// Margen que se reserva del volumen de datos antes de permitir una descarga.
    pub const DISK_RESERVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
    /// De la VRAM total, lo que se puede llenar sin que el driver falle.
    pub const VRAM_BUDGET_FACTOR: f64 = 0.90;

    pub fn ram_budget_bytes(&self) -> u64 {
        (self.total_ram_bytes as f64 * Self::RAM_BUDGET_FACTOR) as u64
    }

    pub fn vram_budget_bytes(&self) -> u64 {
        (self.usable_vram_bytes as f64 * Self::VRAM_BUDGET_FACTOR) as u64
    }

    pub fn disk_budget_bytes(&self) -> u64 {
        self.free_disk_bytes
            .saturating_sub(Self::DISK_RESERVE_BYTES)
    }

    /// Etiqueta corta del equipo para la Home ("16 GB RAM · 8 núcleos · RTX 4060").
    pub fn short_label(&self) -> String {
        let ram = crate::format_bytes(self.total_ram_bytes);
        let gpu = self
            .gpus
            .first()
            .map(|g| g.name.clone())
            .unwrap_or_else(|| "sin GPU dedicada".into());
        format!("{ram} RAM · {} núcleos · {gpu}", self.physical_cores)
    }

    /// Detección real de esta máquina. `data_dir` decide de qué volumen se mide
    /// el espacio libre (donde se guardarán los pesos, no `/`).
    pub fn detect(data_dir: &Path) -> Self {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        sys.refresh_cpu_all();

        let cpu_brand = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "CPU desconocida".into());
        let logical_cores = sys.cpus().len() as u32;
        let physical_cores = sys
            .physical_core_count()
            .map(|n| n as u32)
            .unwrap_or(logical_cores.max(1))
            .max(1);

        let (free_disk_bytes, total_disk_bytes) = disk_for(data_dir);
        let gpus = detect_gpus(sys.total_memory());
        let unified_memory = gpus.iter().any(|g| g.kind == GpuKind::Unified);
        let usable_vram_bytes = gpus
            .iter()
            .map(|g| g.usable_vram_bytes())
            .max()
            .unwrap_or(0);
        let accelerator = pick_accelerator(&gpus);

        Self {
            os: sysinfo::System::name().unwrap_or_else(|| std::env::consts::OS.to_string()),
            os_version: sysinfo::System::os_version().unwrap_or_default(),
            hostname: sysinfo::System::host_name().unwrap_or_default(),
            arch: std::env::consts::ARCH.to_string(),
            cpu_brand,
            physical_cores,
            logical_cores,
            total_ram_bytes: sys.total_memory(),
            available_ram_bytes: sys.available_memory(),
            free_disk_bytes,
            total_disk_bytes,
            gpus,
            accelerator,
            unified_memory,
            usable_vram_bytes,
        }
    }
}

/// Espacio del volumen que contiene `data_dir`: se elige el punto de montaje más
/// específico que sea prefijo de la ruta, no el mayor disco de la máquina.
fn disk_for(data_dir: &Path) -> (u64, u64) {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let best = disks
        .iter()
        .filter(|d| data_dir.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len());
    match best {
        Some(d) => (d.available_space(), d.total_space()),
        None => disks
            .iter()
            .map(|d| (d.available_space(), d.total_space()))
            .max_by_key(|(avail, _)| *avail)
            .unwrap_or((0, 0)),
    }
}

fn pick_accelerator(gpus: &[GpuInfo]) -> Accelerator {
    if gpus.iter().any(|g| g.kind == GpuKind::Unified) {
        return Accelerator::Metal;
    }
    if cfg!(target_os = "macos") {
        // Mac Intel: Metal sigue estando disponible.
        return Accelerator::Metal;
    }
    if gpus
        .iter()
        .any(|g| g.vendor.eq_ignore_ascii_case("NVIDIA") && g.kind == GpuKind::Discrete)
    {
        return Accelerator::Cuda;
    }
    if gpus
        .iter()
        .any(|g| g.vendor.eq_ignore_ascii_case("Qualcomm"))
    {
        return Accelerator::OpenCl;
    }
    if gpus.iter().any(|g| g.kind == GpuKind::Discrete) {
        return Accelerator::Vulkan;
    }
    Accelerator::Cpu
}

fn detect_gpus(total_ram_bytes: u64) -> Vec<GpuInfo> {
    let mut found = run_capture(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ],
    )
    .map(|out| gpu::parse_nvidia_smi(&out))
    .unwrap_or_default();

    if found.is_empty() {
        found = platform_gpus();
    }

    // En memoria unificada la "VRAM" es la RAM del sistema.
    for g in &mut found {
        if g.kind == GpuKind::Unified && g.vram_bytes.is_none() {
            g.vram_bytes = Some(total_ram_bytes);
        }
    }
    found
}

#[cfg(target_os = "windows")]
fn platform_gpus() -> Vec<GpuInfo> {
    run_capture(
        "powershell",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object Name,AdapterRAM,AdapterCompatibility | ConvertTo-Json -Compress",
        ],
    )
    .map(|out| gpu::parse_windows_video_controllers(&out))
    .unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn platform_gpus() -> Vec<GpuInfo> {
    run_capture("system_profiler", &["SPDisplaysDataType", "-json"])
        .map(|out| gpu::parse_system_profiler(&out))
        .unwrap_or_default()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_gpus() -> Vec<GpuInfo> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // "card0" sí; "card0-DP-1" (conectores) no.
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let device = entry.path().join("device");
        let vram = std::fs::read_to_string(device.join("mem_info_vram_total"))
            .ok()
            .and_then(|raw| gpu::parse_sysfs_vram(&raw));
        let label = std::fs::read_to_string(device.join("uevent"))
            .ok()
            .and_then(|raw| {
                raw.lines()
                    .find_map(|l| l.strip_prefix("DRIVER=").map(|s| s.to_string()))
            })
            .unwrap_or_else(|| name.to_string());
        out.push(GpuInfo {
            vendor: gpu::vendor_from_name(&label),
            name: label,
            kind: if vram.is_some() {
                GpuKind::Discrete
            } else {
                GpuKind::Integrated
            },
            vram_bytes: vram,
            source: "sysfs".into(),
        });
    }
    out
}

/// Ejecuta una herramienta del sistema y devuelve su stdout. Que no exista es el
/// caso normal, no un error que haya que registrar.
fn run_capture(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn profile(gpus: Vec<GpuInfo>, ram: u64) -> HardwareProfile {
        let unified = gpus.iter().any(|g| g.kind == GpuKind::Unified);
        let usable = gpus
            .iter()
            .map(|g| g.usable_vram_bytes())
            .max()
            .unwrap_or(0);
        HardwareProfile {
            os: "Test".into(),
            os_version: "1".into(),
            hostname: "pc".into(),
            arch: "x86_64".into(),
            cpu_brand: "Test CPU".into(),
            physical_cores: 8,
            logical_cores: 16,
            total_ram_bytes: ram,
            available_ram_bytes: ram / 2,
            free_disk_bytes: 200 * GIB,
            total_disk_bytes: 512 * GIB,
            accelerator: pick_accelerator(&gpus),
            gpus,
            unified_memory: unified,
            usable_vram_bytes: usable,
        }
    }

    fn gpu(vendor: &str, name: &str, vram: u64, kind: GpuKind) -> GpuInfo {
        GpuInfo {
            vendor: vendor.into(),
            name: name.into(),
            vram_bytes: Some(vram),
            kind,
            source: "test".into(),
        }
    }

    #[test]
    fn detect_never_panics_and_reports_something() {
        let p = HardwareProfile::detect(Path::new("."));
        assert!(p.total_ram_bytes > 0, "sysinfo debe reportar RAM");
        assert!(p.logical_cores >= 1);
        assert!(!p.cpu_brand.is_empty());
    }

    #[test]
    fn nvidia_discrete_selects_cuda() {
        let p = profile(
            vec![gpu(
                "NVIDIA",
                "NVIDIA GeForce RTX 4060",
                8 * GIB,
                GpuKind::Discrete,
            )],
            32 * GIB,
        );
        assert_eq!(p.accelerator, Accelerator::Cuda);
        assert_eq!(p.usable_vram_bytes, 8 * GIB);
    }

    #[test]
    fn integrated_only_falls_back_to_cpu_and_zero_vram() {
        let p = profile(
            vec![gpu(
                "Intel",
                "Intel UHD Graphics 620",
                2 * GIB,
                GpuKind::Integrated,
            )],
            8 * GIB,
        );
        assert_eq!(p.accelerator, Accelerator::Cpu);
        assert_eq!(
            p.usable_vram_bytes, 0,
            "la memoria de una integrada ya está contada en la RAM"
        );
    }

    #[test]
    fn no_gpu_at_all_is_cpu_not_an_error() {
        let p = profile(vec![], 16 * GIB);
        assert_eq!(p.accelerator, Accelerator::Cpu);
        assert_eq!(p.usable_vram_bytes, 0);
    }

    #[test]
    fn unified_memory_selects_metal() {
        let p = profile(
            vec![gpu("Apple", "Apple M3 Pro", 36 * GIB, GpuKind::Unified)],
            36 * GIB,
        );
        assert_eq!(p.accelerator, Accelerator::Metal);
        assert!(p.unified_memory);
    }

    #[test]
    fn budgets_follow_documented_factors() {
        let p = profile(vec![], 16 * GIB);
        assert_eq!(p.ram_budget_bytes(), (16.0 * GIB as f64 * 0.65) as u64);
        assert_eq!(p.disk_budget_bytes(), 200 * GIB - 2 * GIB);
    }

    #[test]
    fn disk_budget_saturates_instead_of_underflowing() {
        let mut p = profile(vec![], 8 * GIB);
        p.free_disk_bytes = 512 * 1024 * 1024; // menos que la reserva
        assert_eq!(p.disk_budget_bytes(), 0);
    }

    #[test]
    fn weight_copies_match_the_documented_criteria() {
        assert_eq!(Accelerator::Cuda.weight_copies(), 2.0);
        assert_eq!(Accelerator::Vulkan.weight_copies(), 2.0);
        assert_eq!(Accelerator::Metal.weight_copies(), 1.15);
        assert_eq!(Accelerator::Cpu.weight_copies(), 1.15);
    }
}
